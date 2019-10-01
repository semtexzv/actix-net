use std::convert::Infallible;
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use actix_service::{NewService, Service};
use futures::future::{ok, Ready};
use futures::{Future, Poll};
use tokio_timer::{delay, Delay};

use super::time::{LowResTime, LowResTimeService};

use pin_project::pin_project;
use std::pin::Pin;
use std::task::Context;

pub struct KeepAlive<R, E, F> {
    f: F,
    ka: Duration,
    time: LowResTime,
    _t: PhantomData<(R, E)>,
}

impl<R, E, F> KeepAlive<R, E, F>
where
    F: Fn() -> E + Clone,
{
    pub fn new(ka: Duration, time: LowResTime, f: F) -> Self {
        KeepAlive {
            f,
            ka,
            time,
            _t: PhantomData,
        }
    }
}

impl<R, E, F> Clone for KeepAlive<R, E, F>
where
    F: Clone,
{
    fn clone(&self) -> Self {
        KeepAlive {
            f: self.f.clone(),
            ka: self.ka,
            time: self.time.clone(),
            _t: PhantomData,
        }
    }
}

impl<R, E, F> NewService for KeepAlive<R, E, F>
where
    F: Fn() -> E + Clone,
{
    type Request = R;
    type Response = R;
    type Error = E;
    type InitError = Infallible;
    type Config = ();
    type Service = KeepAliveService<R, E, F>;
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(KeepAliveService::new(
            self.ka,
            self.time.timer(),
            self.f.clone(),
        ))
    }
}

#[pin_project]
pub struct KeepAliveService<R, E, F> {
    f: F,
    ka: Duration,
    time: LowResTimeService,
    #[pin]
    delay: Delay,
    expire: Instant,
    _t: PhantomData<(R, E)>,
}

impl<R, E, F> KeepAliveService<R, E, F>
where
    F: Fn() -> E,
{
    pub fn new(ka: Duration, time: LowResTimeService, f: F) -> Self {
        let expire = time.now() + ka;
        KeepAliveService {
            f,
            ka,
            time,
            expire,
            delay: delay(expire),
            _t: PhantomData,
        }
    }
}

impl<R, E, F> Service for KeepAliveService<R, E, F>
where
    F: Fn() -> E,
{
    type Request = R;
    type Response = R;
    type Error = E;
    type Future = Ready<Result<R, E>>;

    fn poll_ready(mut self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let mut this = self.project();
        match this.delay.as_mut().poll(ctx) {
            Poll::Ready(_) => {
                let now = this.time.now();
                if *this.expire <= now {
                    Poll::Ready(Err((this.f)()))
                } else {
                    this.delay.reset(*this.expire);
                    let _ = this.delay.poll(ctx);
                    Poll::Ready(Ok(()))
                }
            }
            Poll::Pending => Poll::Ready(Ok(())),
        }
    }

    fn call(&mut self, req: R) -> Self::Future {
        self.expire = self.time.now() + self.ka;
        ok(req)
    }
}
