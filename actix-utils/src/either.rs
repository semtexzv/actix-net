//! Contains `Either` service and related types and functions.
use actix_service::{IntoNewService, NewService, Service};
use futures::{future, ready, Future, Poll};

/// Combine two different service types into a single type.
///
/// Both services must be of the same request, response, and error types.
/// `EitherService` is useful for handling conditional branching in service
/// middleware to different inner service types.
pub struct EitherService<A, B> {
    left: A,
    right: B,
}

impl<A: Clone, B: Clone> Clone for EitherService<A, B> {
    fn clone(&self) -> Self {
        EitherService {
            left: self.left.clone(),
            right: self.right.clone(),
        }
    }
}
/*
impl<A, B> Service for EitherService<A, B>
where
    A: Service,
    B: Service<Response = A::Response, Error = A::Error>,
{
    type Request = either::Either<A::Request, B::Request>;
    type Response = A::Response;
    type Error = A::Error;
    type Future = future::Either<A::Future, B::Future>;

    fn poll_ready(&mut self) -> Poll<(), Self::Error> {
        let left = self.left.poll_ready()?;
        let right = self.right.poll_ready()?;

        if left.is_ready() && right.is_ready() {
            Ok(Poll::Ready(()))
        } else {
            Ok(Poll::Pending)
        }
    }

    fn call(&mut self, req: either::Either<A::Request, B::Request>) -> Self::Future {
        match req {
            either::Either::Left(req) => future::Either::A(self.left.call(req)),
            either::Either::Right(req) => future::Either::B(self.right.call(req)),
        }
    }
}
*/
/// Combine two different new service types into a single service.
pub struct Either<A, B> {
    left: A,
    right: B,
}

impl<A, B> Either<A, B> {
    pub fn new<F1, F2>(srv_a: F1, srv_b: F2) -> Either<A, B>
    where
        A: NewService,
        B: NewService<
            Config = A::Config,
            Response = A::Response,
            Error = A::Error,
            InitError = A::InitError,
        >,
        F1: IntoNewService<A>,
        F2: IntoNewService<B>,
    {
        Either {
            left: srv_a.into_new_service(),
            right: srv_b.into_new_service(),
        }
    }
}

impl<A, B> NewService for Either<A, B>
where
    A: NewService,
    B: NewService<
        Config = A::Config,
        Response = A::Response,
        Error = A::Error,
        InitError = A::InitError,
    >,
{
    type Request = either::Either<A::Request, B::Request>;
    type Response = A::Response;
    type Error = A::Error;
    type InitError = A::InitError;
    type Config = A::Config;
    type Service = EitherService<A::Service, B::Service>;
    type Future = EitherNewService<A, B>;

    fn new_service(&self, cfg: &A::Config) -> Self::Future {
        EitherNewService {
            left: None,
            right: None,
            left_fut: self.left.new_service(cfg),
            right_fut: self.right.new_service(cfg),
        }
    }
}

impl<A: Clone, B: Clone> Clone for Either<A, B> {
    fn clone(&self) -> Self {
        Self {
            left: self.left.clone(),
            right: self.right.clone(),
        }
    }
}

#[doc(hidden)]
pub struct EitherNewService<A: NewService, B: NewService> {
    left: Option<A::Service>,
    right: Option<B::Service>,
    left_fut: A::Future,
    right_fut: B::Future,
}

impl<A, B> Future for EitherNewService<A, B>
where
    A: NewService,
    B: NewService<Response = A::Response, Error = A::Error, InitError = A::InitError>,
{
    type Output = Result<EitherService<A::Service, B::Service>,A::InitError>;
    /*
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        if self.left.is_none() {
            self.left = Some(ready!(self.left_fut.poll()));
        }
        if self.right.is_none() {
            self.right = Some(ready!(self.right_fut.poll()));
        }

        if self.left.is_some() && self.right.is_some() {
            Ok(Poll::Ready(EitherService {
                left: self.left.take().unwrap(),
                right: self.right.take().unwrap(),
            }))
        } else {
            Ok(Poll::Pending)
        }
    }
    */
}
