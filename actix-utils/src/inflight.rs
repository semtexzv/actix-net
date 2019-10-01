use std::convert::Infallible;

use actix_service::{IntoService, Service, Transform};
use futures::future::{ok, Ready};
use futures::{Future, Poll};

use super::counter::{Counter, CounterGuard};
use std::pin::Pin;
use std::task::Context;
use pin_project::pin_project;

/// InFlight - new service for service that can limit number of in-flight
/// async requests.
///
/// Default number of in-flight requests is 15
pub struct InFlight {
    max_inflight: usize,
}

impl InFlight {
    pub fn new(max: usize) -> Self {
        Self { max_inflight: max }
    }
}

impl Default for InFlight {
    fn default() -> Self {
        Self::new(15)
    }
}

impl<S: Service> Transform<S> for InFlight {
    type Request = S::Request;
    type Response = S::Response;
    type Error = S::Error;
    type InitError = Infallible;
    type Transform = InFlightService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(InFlightService::new(self.max_inflight, service))
    }
}

#[pin_project]
pub struct InFlightService<S> {
    count: Counter,
    #[pin]
    service: S,
}

impl<S> InFlightService<S>
where
    S: Service,
{
    pub fn new<U>(max: usize, service: U) -> Self
    where
        U: IntoService<S>,
    {
        Self {
            count: Counter::new(max),
            service: service.into_service(),
        }
    }
}

impl<T> Service for InFlightService<T>
where
    T: Service,
{
    type Request = T::Request;
    type Response = T::Response;
    type Error = T::Error;
    type Future = InFlightServiceResponse<T>;

    fn poll_ready(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project();
        if let Poll::Pending = this.service.poll_ready(ctx)? {
            Poll::Pending
        } else if !this.count.available() {
            log::trace!("InFlight limit exceeded");
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }
    }

    fn call(&mut self, req: T::Request) -> Self::Future {
        // TODO: Another place where we might wanna pass ctx into call method or rethink the design
        InFlightServiceResponse {
            fut: self.service.call(req),
            _guard: unimplemented!() //self.count.get(),
        }
    }
}

#[doc(hidden)]
#[pin_project]
pub struct InFlightServiceResponse<T: Service> {
    #[pin]
    fut: T::Future,
    _guard: CounterGuard,
}

impl<T: Service> Future for InFlightServiceResponse<T> {
    type Output = Result<T::Response,T::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project().fut.poll(cx)
    }
}


#[cfg(test)]
mod tests {
    use futures::future::lazy;
    use futures::{Poll};

    use std::time::Duration;

    use super::*;
    use actix_service::blank::{Blank, BlankNewService};
    use actix_service::{NewService, Service, ServiceExt};

    struct SleepService(Duration);

    impl Service for SleepService {
        type Request = ();
        type Response = ();
        type Error = ();
        type Future = Box<dyn Future<Item = (), Error = ()>>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            Ok(Poll::Ready(()))
        }

        fn call(&mut self, _: ()) -> Self::Future {
            Box::new(tokio_timer::sleep(self.0).map_err(|_| ()))
        }
    }

    #[test]
    fn test_transform() {
        let wait_time = Duration::from_millis(50);
        let _ = actix_rt::System::new("test").block_on(lazy(|| {
            let mut srv =
                Blank::new().and_then(InFlightService::new(1, SleepService(wait_time)));
            assert_eq!(srv.poll_ready(), Ok(Poll::Ready(())));

            let mut res = srv.call(());
            let _ = res.poll();
            assert_eq!(srv.poll_ready(), Ok(Poll::Pending));

            drop(res);
            assert_eq!(srv.poll_ready(), Ok(Poll::Ready(())));

            Ok::<_, ()>(())
        }));
    }

    #[test]
    fn test_newtransform() {
        let wait_time = Duration::from_millis(50);
        let _ = actix_rt::System::new("test").block_on(lazy(|| {
            let srv =
                BlankNewService::new().apply(InFlight::new(1), || Ok(SleepService(wait_time)));

            if let Poll::Ready(mut srv) = srv.new_service(&()).poll().unwrap() {
                assert_eq!(srv.poll_ready(), Ok(Poll::Ready(())));

                let mut res = srv.call(());
                let _ = res.poll();
                assert_eq!(srv.poll_ready(), Ok(Poll::Pending));

                drop(res);
                assert_eq!(srv.poll_ready(), Ok(Poll::Ready(())));
            } else {
                panic!()
            }

            Ok::<_, ()>(())
        }));
    }
}
