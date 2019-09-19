use std::marker::PhantomData;
use std::pin::Pin;
use std::task::Context;
use futures::{Future, Poll, TryFuture};

use super::{IntoNewService, IntoService, NewService, Service};
use crate::cell::Cell;

use pin_project::pin_project;


/// `Apply` service combinator
#[pin_project]
pub struct AndThenApply<A, B, F, OutFut, Res>
    where
        A: Service,
        B: Service<Error=A::Error>,
        F: FnMut(A::Response, &mut B) -> OutFut,
        OutFut: Future<Output=Result<Res, A::Error>>
{
    #[pin]
    a: A,
    #[pin]
    b: Cell<B>,
    #[pin]
    f: Cell<F>,
    r: PhantomData<(OutFut, Res, )>,
}

impl<A, B, F, OutFut, Res> AndThenApply<A, B, F, OutFut, Res>
    where
        A: Service,
        B: Service<Error=A::Error>,
        F: FnMut(A::Response, &mut B) -> OutFut,
        OutFut: Future<Output=Result<Res, A::Error>>
{
    /// Create new `Apply` combinator
    pub fn new<A1: IntoService<A>, B1: IntoService<B>>(a: A1, b: B1, f: F) -> Self {
        Self {
            f: Cell::new(f),
            a: a.into_service(),
            b: Cell::new(b.into_service()),
            r: PhantomData,
        }
    }
}

impl<A, B, F, OutFut, Res> Clone for AndThenApply<A, B, F, OutFut, Res>
    where
        A: Service + Clone,
        B: Service<Error=A::Error>,
        F: FnMut(A::Response, &mut B) -> OutFut,
        OutFut: Future<Output=Result<Res, A::Error>>
{
    fn clone(&self) -> Self {
        AndThenApply {
            a: self.a.clone(),
            b: self.b.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<A, B, F, OutFut, Res> Service for AndThenApply<A, B, F, OutFut, Res>
    where
        A: Service,
        B: Service<Error=A::Error>,
        F: FnMut(A::Response, &mut B) -> OutFut,
        OutFut: Future<Output=Result<Res, A::Error>>
{
    type Request = A::Request;
    type Response = Res;
    type Error = A::Error;
    type Future = AndThenApplyFuture<A, B, F, OutFut, Res>;

    fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let this = self.project_into();
        let not_ready = !this.a.poll_ready(ctx)?.is_ready();
        if !this.b.get_pin().poll_ready(ctx).is_ready() || not_ready {
            Poll::Pending
        } else {
            Poll::Ready(Ok(()))
        }

    }


    fn call(&mut self, req: A::Request) -> Self::Future {
        AndThenApplyFuture {
            b: self.b.clone(),
            f: self.f.clone(),
            fut_b: None,
            fut_a: Some(self.a.call(req)),
        }
    }
}

#[pin_project]
pub struct AndThenApplyFuture<A, B, F, OutFut, Res>
    where
        A: Service,
        B: Service<Error=A::Error>,
        F: FnMut(A::Response, &mut B) -> OutFut,
        OutFut: Future<Output=Result<Res, A::Error>>
{
    b: Cell<B>,
    f: Cell<F>,
    #[pin]
    fut_a: Option<A::Future>,
    #[pin]
    fut_b: Option<OutFut>,
}


impl<A, B, F, OutFut, Res> Future for AndThenApplyFuture<A, B, F, OutFut, Res>
    where
        A: Service,
        B: Service<Error=A::Error>,
        F: FnMut(A::Response, &mut B) -> OutFut,
        OutFut: Future<Output=Result<Res, A::Error>>
{
    type Output = Result<Res, A::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        if let Some(fut) = this.fut_b.as_mut().as_pin_mut() {
            return fut.poll(cx).map_err(|e| e.into());
        }

        match this.fut_a.as_mut().as_pin_mut().expect("Bug in actix-service").poll(cx)? {
            Poll::Ready(resp) => {
                this.fut_a.set(None);
                this.fut_b.set(Some((&mut *this.f.get_mut())(resp, this.b.get_mut())));
                self.poll(cx)
            },
            Poll::Pending => Poll::Pending
        }
    }
}

/// `ApplyNewService` new service combinator
pub struct AndThenApplyNewService<A, B, F, OutFut, Res> {
    a: A,
    b: B,
    f: Cell<F>,
    r: PhantomData<(OutFut, Res, )>,
}

impl<A, B, F, OutFut, Res> AndThenApplyNewService<A, B, F, OutFut, Res>
    where
        A: NewService,
        B: NewService<Config=A::Config, Error=A::Error, InitError=A::InitError>,
        F: FnMut(A::Response, &mut B::Service) -> OutFut,
        OutFut: Future<Output=Result<Res, A::Error>>
{
    /// Create new `ApplyNewService` new service instance
    pub fn new<A1: IntoNewService<A>, B1: IntoNewService<B>>(a: A1, b: B1, f: F) -> Self {
        Self {
            f: Cell::new(f),
            a: a.into_new_service(),
            b: b.into_new_service(),
            r: PhantomData,
        }
    }
}

impl<A, B, F, OutFut, Res> Clone for AndThenApplyNewService<A, B, F, OutFut, Res>
    where
        A: Clone,
        B: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            b: self.b.clone(),
            f: self.f.clone(),
            r: PhantomData,
        }
    }
}

impl<A, B, F, OutFut,Res> NewService for AndThenApplyNewService<A, B, F, OutFut, Res>
    where
        A: NewService,
        B: NewService<Config=A::Config, Error=A::Error, InitError=A::InitError>,
        F: FnMut(A::Response, &mut B::Service) -> OutFut,
        OutFut: Future<Output=Result<Res, A::Error>>
{
    type Request = A::Request;
    type Response = Res;
    type Error = A::Error;
    type Service = AndThenApply<A::Service, B::Service, F, OutFut, Res>;
    type Config = A::Config;
    type InitError = A::InitError;
    type Future = AndThenApplyNewServiceFuture<A, B, F, OutFut, Res>;

    fn new_service(&self, cfg: &A::Config) -> Self::Future {
        AndThenApplyNewServiceFuture {
            a: None,
            b: None,
            f: self.f.clone(),
            fut_a: self.a.new_service(cfg),//.into_future(),
            fut_b: self.b.new_service(cfg),//.into_future(),
        }
    }
}
#[pin_project]
pub struct AndThenApplyNewServiceFuture<A, B, F, OutFut, Res>
    where
        A: NewService,
        B: NewService<Error=A::Error, InitError=A::InitError>,
        F: FnMut(A::Response, &mut B::Service) -> OutFut,
        OutFut: Future<Output=Result<Res, A::Error>>
{
    #[pin]
    fut_b: B::Future,
    #[pin]
    fut_a: A::Future,
    f: Cell<F>,
    a: Option<A::Service>,
    b: Option<B::Service>,
}

impl<A, B, F, OutFut, Res> Future for AndThenApplyNewServiceFuture<A, B, F, OutFut, Res>
    where
        A: NewService,
        B: NewService<Error=A::Error, InitError=A::InitError>,
        F: FnMut(A::Response, &mut B::Service) -> OutFut,
        OutFut: Future<Output=Result<Res, A::Error>>
{
    type Output = Result<AndThenApply<A::Service, B::Service, F, OutFut, Res>, A::InitError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();

        if this.a.is_none() {
            if let Poll::Ready(service) = this.fut_a.poll(cx)? {
                *this.a = Some(service);
            }
        }

        if this.b.is_none() {
            if let Poll::Ready(service) = this.fut_b.poll(cx)? {
                *this.b = Some(service);
            }
        }

        if this.a.is_some() && this.b.is_some() {
            Poll::Ready(Ok(AndThenApply {
                f: this.f.clone(),
                a: this.a.take().unwrap(),
                b: Cell::new(this.b.take().unwrap()),
                r: PhantomData,
            }))
        } else {
            Poll::Pending
        }
    }

}


#[cfg(test)]
mod tests {
    use futures::future::{ok, FutureResult};
    use futures::{Async, Future, Poll};

    use crate::blank::{Blank, BlankNewService};
    use crate::{NewService, Service, ServiceExt};

    #[derive(Clone)]
    struct Srv;

    impl Service for Srv {
        type Request = ();
        type Response = ();
        type Error = ();
        type Future = FutureResult<(), ()>;

        fn poll_ready(&mut self) -> Poll<(), Self::Error> {
            Ok(Async::Ready(()))
        }

        fn call(&mut self, _: ()) -> Self::Future {
            ok(())
        }
    }

    #[test]
    fn test_call() {
        let mut srv = Blank::new().apply_fn(Srv, |req: &'static str, srv| {
            srv.call(()).map(move |res| (req, res))
        });
        assert!(srv.poll_ready().is_ok());
        let res = srv.call("srv").poll();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Async::Ready(("srv", ())));
    }

    #[test]
    fn test_new_service() {
        let new_srv = BlankNewService::new_unit().apply_fn(
            || Ok(Srv),
            |req: &'static str, srv| srv.call(()).map(move |res| (req, res)),
        );
        if let Async::Ready(mut srv) = new_srv.new_service(&()).poll().unwrap() {
            assert!(srv.poll_ready().is_ok());
            let res = srv.call("srv").poll();
            assert!(res.is_ok());
            assert_eq!(res.unwrap(), Async::Ready(("srv", ())));
        } else {
            panic!()
        }
    }
}
