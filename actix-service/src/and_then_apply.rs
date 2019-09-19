use std::rc::Rc;

use futures::{Future, Poll};

use crate::and_then::AndThen;
use crate::from_err::FromErr;
use crate::{NewService, Transform};

use pin_project::pin_project;
use std::task::Context;
use std::pin::Pin;

/// `Apply` new service combinator
pub struct AndThenTransform<T, A, B> {
    a: A,
    b: B,
    t: Rc<T>,
}

impl<T, A, B> AndThenTransform<T, A, B>
where
    A: NewService,
    B: NewService<Config = A::Config, InitError = A::InitError>,
    T: Transform<B::Service, Request = A::Response, InitError = A::InitError>,
    T::Error: From<A::Error>,
{
    /// Create new `ApplyNewService` new service instance
    pub fn new(t: T, a: A, b: B) -> Self {
        Self {
            a,
            b,
            t: Rc::new(t),
        }
    }
}

impl<T, A, B> Clone for AndThenTransform<T, A, B>
where
    A: Clone,
    B: Clone,
{
    fn clone(&self) -> Self {
        Self {
            a: self.a.clone(),
            b: self.b.clone(),
            t: self.t.clone(),
        }
    }
}

impl<T, A, B> NewService for AndThenTransform<T, A, B>
where
    A: NewService,
    B: NewService<Config = A::Config, InitError = A::InitError>,
    T: Transform<B::Service, Request = A::Response, InitError = A::InitError>,
    T::Error: From<A::Error>,
{
    type Request = A::Request;
    type Response = T::Response;
    type Error = T::Error;

    type Config = A::Config;
    type InitError = T::InitError;
    type Service = AndThen<FromErr<A::Service, T::Error>, T::Transform>;
    type Future = AndThenTransformFuture<T, A, B>;

    fn new_service(&self, cfg: &A::Config) -> Self::Future {
        AndThenTransformFuture {
            a: None,
            t: None,
            t_cell: self.t.clone(),
            fut_a: self.a.new_service(cfg),
            fut_b: self.b.new_service(cfg),
            fut_t: None,
        }
    }
}

#[pin_project]
pub struct AndThenTransformFuture<T, A, B>
where
    A: NewService,
    B: NewService<InitError = A::InitError>,
    T: Transform<B::Service, Request = A::Response, InitError = A::InitError>,
    T::Error: From<A::Error>,
{
    #[pin]
    fut_a: A::Future,
    #[pin]
    fut_b: B::Future,
    #[pin]
    fut_t: Option<T::Future>,
    a: Option<A::Service>,
    t: Option<T::Transform>,
    t_cell: Rc<T>,
}

impl<T, A, B> Future for AndThenTransformFuture<T, A, B>
where
    A: NewService,
    B: NewService<InitError = A::InitError>,
    T: Transform<B::Service, Request = A::Response, InitError = A::InitError>,
    T::Error: From<A::Error>,
{

    type Output = Result<AndThen<FromErr<A::Service, T::Error>, T::Transform>,T::InitError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project_into();

        if this.fut_t.is_none() {
            if let Poll::Ready(svc) = this.fut_b.poll(cx)? {
                this.fut_t.set(Some(this.t_cell.new_transform(svc)))
            }
        }

        if this.a.is_none() {
            if let Poll::Ready(svc) = this.fut_a.poll(cx)? {
                *this.a = Some(svc)
            }
        }

        if let Some(fut) = this.fut_t.as_pin_mut() {
            if let Poll::Ready(transform) = fut.poll(cx)? {
                *this.t = Some(transform)
            }
        }

        if this.a.is_some() && this.t.is_some() {
            Poll::Ready(Ok(AndThen::new(
                FromErr::new(this.a.take().unwrap()),
                this.t.take().unwrap(),
            )))
        } else {
            Poll::Pending
        }

    }



    /*
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.fut_t.is_none() {
            if let Poll::Ready(service) = self.fut_b.poll()? {
                self.fut_t = Some(self.t_cell.new_transform(service));
            }
        }

        if self.a.is_none() {
            if let Poll::Ready(service) = self.fut_a.poll()? {
                self.a = Some(service);
            }
        }

        if let Some(ref mut fut) = self.fut_t {
            if let Poll::Ready(transform) = fut.poll()? {
                self.t = Some(transform);
            }
        }

        if self.a.is_some() && self.t.is_some() {
            Ok(Poll::Ready(AndThen::new(
                FromErr::new(self.a.take().unwrap()),
                self.t.take().unwrap(),
            )))
        } else {
            Ok(Poll::Pending)
        }
    }
    */
}

#[cfg(test)]
mod tests {
    use futures::future::{ok, Ready};
    use futures::{Future, Poll};

    use crate::{IntoNewService, IntoService, NewService, Service, ServiceExt};
    use std::pin::Pin;
    use std::task::Context;

    #[derive(Clone)]
    struct Srv;
    impl Service for Srv {
        type Request = ();
        type Response = ();
        type Error = ();
        type Future = Ready<Result<(), ()>>;

        fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Err(()))
        }

        fn call(&mut self, _: ()) -> Self::Future {
            ok(())
        }
    }

    #[test]
    fn test_apply() {
        let blank = |req| Ok(req);

        let mut srv = blank
            .into_service()
            .apply_fn(Srv, |req: &'static str, srv: &mut Srv| {
                srv.call(()).map(move |res| (req, res))
            });
        assert!(srv.poll_ready().is_ok());
        let res = srv.call("srv").poll();
        assert!(res.is_ok());
        assert_eq!(res.unwrap(), Poll::Ready(("srv", ())));
    }

    #[test]
    fn test_new_service() {
        let blank = || Ok::<_, ()>((|req| Ok(req)).into_service());

        let new_srv = blank.into_new_service().apply(
            |req: &'static str, srv: &mut Srv| srv.call(()).map(move |res| (req, res)),
            || Ok(Srv),
        );
        if let Poll::Ready(mut srv) = new_srv.new_service(&()).poll().unwrap() {
            assert!(srv.poll_ready().is_ok());
            let res = srv.call("srv").poll();
            assert!(res.is_ok());
            assert_eq!(res.unwrap(), Poll::Ready(("srv", ())));
        } else {
            panic!()
        }
    }
}
