//! Service that applies a timeout to requests.
//!
//! If the response does not complete within the specified timeout, the response
//! will be aborted.
use std::fmt;
use std::marker::PhantomData;
use std::time::Duration;

use actix_service::{IntoService, Service, Transform};
use futures::future::{ok, Ready};
use futures::{ready, Future, Poll};
use tokio_timer::{clock, delay, Delay};
use std::pin::Pin;
use std::task::Context;

use pin_project::pin_project;

/// Applies a timeout to requests.
#[derive(Debug)]
pub struct Timeout<E = ()> {
    timeout: Duration,
    _t: PhantomData<E>,
}

/// Timeout error
pub enum TimeoutError<E> {
    /// Service error
    Service(E),
    /// Service call timeout
    Timeout,
}

impl<E> From<E> for TimeoutError<E> {
    fn from(err: E) -> Self {
        TimeoutError::Service(err)
    }
}

impl<E: fmt::Debug> fmt::Debug for TimeoutError<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TimeoutError::Service(e) => write!(f, "TimeoutError::Service({:?})", e),
            TimeoutError::Timeout => write!(f, "TimeoutError::Timeout"),
        }
    }
}

impl<E: fmt::Display> fmt::Display for TimeoutError<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            TimeoutError::Service(e) => e.fmt(f),
            TimeoutError::Timeout => write!(f, "Service call timeout"),
        }
    }
}

impl<E: PartialEq> PartialEq for TimeoutError<E> {
    fn eq(&self, other: &TimeoutError<E>) -> bool {
        match self {
            TimeoutError::Service(e1) => match other {
                TimeoutError::Service(e2) => e1 == e2,
                TimeoutError::Timeout => false,
            },
            TimeoutError::Timeout => match other {
                TimeoutError::Service(_) => false,
                TimeoutError::Timeout => true,
            },
        }
    }
}

impl<E> Timeout<E> {
    pub fn new(timeout: Duration) -> Self {
        Timeout {
            timeout,
            _t: PhantomData,
        }
    }
}

impl<E> Clone for Timeout<E> {
    fn clone(&self) -> Self {
        Timeout::new(self.timeout)
    }
}

impl<S, E> Transform<S> for Timeout<E>
    where
        S: Service,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = TimeoutError<S::Error>;
    type InitError = E;
    type Transform = TimeoutService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(TimeoutService {
            service,
            timeout: self.timeout,
        })
    }
}

/// Applies a timeout to requests.
#[pin_project]
#[derive(Debug, Clone)]
pub struct TimeoutService<S> {
    #[pin]
    service: S,
    timeout: Duration,
}

impl<S> TimeoutService<S>
    where
        S: Service,
{
    pub fn new<U>(timeout: Duration, service: U) -> Self
        where
            U: IntoService<S>,
    {
        TimeoutService {
            timeout,
            service: service.into_service(),
        }
    }
}

impl<S> Service for TimeoutService<S>
    where
        S: Service,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = TimeoutError<S::Error>;
    type Future = TimeoutServiceResponse<S>;

    fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project().service.poll_ready(ctx).map_err(TimeoutError::Service)
    }

    fn call(&mut self, request: S::Request) -> Self::Future {
        TimeoutServiceResponse {
            fut: self.service.call(request),
            sleep: delay(clock::now() + self.timeout),
        }
    }
}

/// `TimeoutService` response future
#[pin_project]
#[derive(Debug)]
pub struct TimeoutServiceResponse<T: Service> {
    #[pin]
    fut: T::Future,
    #[pin]
    sleep: Delay,
}

impl<T> Future for TimeoutServiceResponse<T>
    where
        T: Service,
{
    type Output = Result<T::Response, TimeoutError<T::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        // First, try polling the future
        match this.fut.poll(cx) {
            Poll::Ready(Ok(v)) => return Poll::Ready(Ok(v)),
            Poll::Ready(Err(e)) => return Poll::Ready(Err(TimeoutError::Service(e))),
            Poll::Pending => {}
        }

        // Now check the sleep
        let () = ready!(this.sleep.poll(cx));
        Poll::Ready(Err(TimeoutError::Timeout))
    }
}

#[cfg(test)]
mod tests {
    use futures::future::{lazy, LocalBoxFuture};
    use futures::{Poll, FutureExt};

    use std::time::Duration;

    use super::*;
    use actix_service::blank::{Blank, BlankNewService};
    use actix_service::{NewService, Service, ServiceExt};

    struct SleepService(Duration);

    impl Service for SleepService {
        type Request = ();
        type Response = ();
        type Error = ();
        type Future = LocalBoxFuture<'static, Result<(), ()>>;

        fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }


        fn call(&mut self, _: ()) -> Self::Future {
            tokio_timer::sleep(self.0).map(Ok).boxed_local()
        }
    }

    #[test]
    fn test_success() {
        let resolution = Duration::from_millis(100);
        let wait_time = Duration::from_millis(50);

        let res = actix_rt::System::new("test").block_on(async {
            let mut timeout = Blank::default()
                .and_then(TimeoutService::new(resolution, SleepService(wait_time)));
            timeout.call(()).await
        });
        assert_eq!(res, Ok(()));
    }

    #[test]
    fn test_timeout() {
        let resolution = Duration::from_millis(100);
        let wait_time = Duration::from_millis(150);

        let res = actix_rt::System::new("test").block_on(async {
            let mut timeout = Blank::default()
                .and_then(TimeoutService::new(resolution, SleepService(wait_time)));
            timeout.call(()).await
        });
        assert_eq!(res, Err(TimeoutError::Timeout));
    }

    #[test]
    fn test_timeout_newservice() {
        let resolution = Duration::from_millis(100);
        let wait_time = Duration::from_millis(150);

        let res = actix_rt::System::new("test").block_on(async {
            let timeout = BlankNewService::<(), (), ()>::default()
                .apply(Timeout::new(resolution), || ok(SleepService(wait_time)));
            let mut to = timeout.new_service(&()).await.unwrap();
            to.call(()).await
        });
        assert_eq!(res, Err(TimeoutError::Timeout));
    }
}
