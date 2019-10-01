//! Contains `Either` service and related types and functions.
use std::pin::Pin;
use std::task::Context;
use futures::{future, ready, Future, Poll};

use actix_service::{IntoNewService, NewService, Service, IntoFuture};
use pin_project::pin_project;

/// Combine two different service types into a single type.
///
/// Both services must be of the same request, response, and error types.
/// `EitherService` is useful for handling conditional branching in service
/// middleware to different inner service types.
#[pin_project]
pub struct EitherService<A, B> {
    #[pin]
    left: A,
    #[pin]
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

impl<A, B> Service for EitherService<A, B>
where
    A: Service,
    B: Service<Response = A::Response, Error = A::Error>,
{
    type Request = either::Either<A::Request, B::Request>;
    type Response = A::Response;
    type Error = A::Error;
    type Future = future::Either<A::Future, B::Future>;

    fn poll_ready(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let mut this = self.project();
        let left = this.left.poll_ready(ctx)?;
        let right = this.right.poll_ready(ctx)?;

        if left.is_ready() && right.is_ready() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }


    fn call(&mut self, req: either::Either<A::Request, B::Request>) -> Self::Future {
        match req {
            either::Either::Left(req) => future::Either::Left(self.left.call(req)),
            either::Either::Right(req) => future::Either::Right(self.right.call(req)),
        }
    }
}

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
#[pin_project]
pub struct EitherNewService<A: NewService, B: NewService> {
    left: Option<A::Service>,
    right: Option<B::Service>,
    #[pin]
    left_fut: <A::Future as IntoFuture>::Future,
    #[pin]
    right_fut: <B::Future as IntoFuture>::Future,
}

impl<A, B> Future for EitherNewService<A, B>
where
    A: NewService,
    B: NewService<Response = A::Response, Error = A::Error, InitError = A::InitError>,
{
    type Output = Result<EitherService<A::Service, B::Service>,A::InitError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        if this.left.is_none() {
            *this.left = Some(ready!(this.left_fut.poll(cx))?);
        }
        if this.right.is_none() {
            *this.right = Some(ready!(this.right_fut.poll(cx))?);
        }

        if this.left.is_some() && this.right.is_some() {
            Poll::Ready(Ok(EitherService {
                left: this.left.take().unwrap(),
                right: this.right.take().unwrap(),
            }))
        } else {
            Poll::Pending
        }
    }
}
