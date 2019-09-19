use std::marker::PhantomData;

use futures::{Future, Poll};

use super::{NewService, Service};
use std::pin::Pin;
use std::task::Context;

use pin_project::pin_project;
/// Service for the `from_err` combinator, changing the error type of a service.
///
/// This is created by the `ServiceExt::from_err` method.
#[pin_project]
pub struct FromErr<A, E> {
    #[pin]
    service: A,
    f: PhantomData<E>,
}

impl<A, E> FromErr<A, E> {
    pub(crate) fn new(service: A) -> Self
    where
        A: Service,
        E: From<A::Error>,
    {
        FromErr {
            service,
            f: PhantomData,
        }
    }
}

impl<A, E> Clone for FromErr<A, E>
where
    A: Clone,
{
    fn clone(&self) -> Self {
        FromErr {
            service: self.service.clone(),
            f: PhantomData,
        }
    }
}

impl<A, E> Service for FromErr<A, E>
where
    A: Service,
    E: From<A::Error>,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = E;
    type Future = FromErrFuture<A, E>;

    fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.project_into().service.poll_ready(ctx).map_err(E::from)
    }

    fn call(&mut self, req: A::Request) -> Self::Future {
        FromErrFuture {
            fut: self.service.call(req),
            f: PhantomData,
        }
    }
}
#[pin_project]
pub struct FromErrFuture<A: Service, E> {
    #[pin]
    fut: A::Future,
    f: PhantomData<E>,
}

impl<A, E> Future for FromErrFuture<A, E>
where
    A: Service,
    E: From<A::Error>,
{

    type Output = Result<A::Response,E>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.project_into().fut.poll(cx).map_err(E::from)
    }
}

/// NewService for the `from_err` combinator, changing the type of a new
/// service's error.
///
/// This is created by the `NewServiceExt::from_err` method.
pub struct FromErrNewService<A, E> {
    a: A,
    e: PhantomData<E>,
}

impl<A, E> FromErrNewService<A, E> {
    /// Create new `FromErr` new service instance
    pub fn new(a: A) -> Self
    where
        A: NewService,
        E: From<A::Error>,
    {
        Self { a, e: PhantomData }
    }
}

impl<A, E> Clone for FromErrNewService<A, E>
where
    A: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            e: PhantomData,
        }
    }
}

impl<A, E> NewService for FromErrNewService<A, E>
where
    A: NewService,
    E: From<A::Error>,
{
    type Request = A::Request;
    type Response = A::Response;
    type Error = E;

    type Config = A::Config;
    type Service = FromErr<A::Service, E>;
    type InitError = A::InitError;
    type Future = FromErrNewServiceFuture<A, E>;

    fn new_service(&self, cfg: &A::Config) -> Self::Future {
        FromErrNewServiceFuture {
            fut: self.a.new_service(cfg),
            e: PhantomData,
        }
    }
}
#[pin_project]
pub struct FromErrNewServiceFuture<A, E>
where
    A: NewService,
    E: From<A::Error>,
{
    #[pin]
    fut: A::Future,
    e: PhantomData<E>,
}

impl<A, E> Future for FromErrNewServiceFuture<A, E>
where
    A: NewService,
    E: From<A::Error>,
{
    type Output = Result<FromErr<A::Service,E>,A::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Poll::Ready(svc) = self.project_into().fut.poll(cx)? {
            Poll::Ready(Ok(FromErr::new(svc)))
        } else {
            Poll::Pending
        }
    }

    /*
        fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
            if let Async::Ready(service) = self.fut.poll()? {
                Ok(Async::Ready(FromErr::new(service)))
            } else {
                Ok(Async::NotReady)
            }
        }
        */
}

#[cfg(test)]
mod tests {
    use futures::future::{err, FutureResult};

    use super::*;
    use crate::{IntoNewService, NewService, Service, ServiceExt};

    struct Srv;
    impl Service for Srv {
        type Request = ();
        type Response = ();
        type Error = ();
        type Future = FutureResult<(), ()>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            Err(())
        }

        fn call(&mut self, _: ()) -> Self::Future {
            err(())
        }
    }

    #[derive(Debug, PartialEq)]
    struct Error;

    impl From<()> for Error {
        fn from(_: ()) -> Self {
            Error
        }
    }

    #[test]
    fn test_poll_ready() {
        let mut srv = Srv.from_err::<Error>();
        let res = srv.poll_ready();
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), Error);
    }

    #[test]
    fn test_call() {
        let mut srv = Srv.from_err::<Error>();
        let res = srv.call(()).poll();
        assert!(res.is_err());
        assert_eq!(res.err().unwrap(), Error);
    }

    #[test]
    fn test_new_service() {
        let blank = || Ok::<_, ()>(Srv);
        let new_srv = blank.into_new_service().from_err::<Error>();
        if let Async::Ready(mut srv) = new_srv.new_service(&()).poll().unwrap() {
            let res = srv.call(()).poll();
            assert!(res.is_err());
            assert_eq!(res.err().unwrap(), Error);
        } else {
            panic!()
        }
    }
}
