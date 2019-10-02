use std::collections::VecDeque;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;

use actix_service::{IntoService, Service, Transform};
use futures::future::{ok, ready, Ready};
use futures::task::AtomicWaker;
use futures::channel::oneshot;
use futures::{ready, Future, Poll, FutureExt};
use std::pin::Pin;
use std::task::Context;


use pin_project::pin_project;
use std::convert::Infallible;

struct Record<I, E> {
    rx: oneshot::Receiver<Result<I, E>>,
    tx: oneshot::Sender<Result<I, E>>,
}

/// Timeout error
pub enum InOrderError<E> {
    /// Service error
    Service(E),
    /// Service call dropped
    Disconnected,
}

impl<E> From<E> for InOrderError<E> {
    fn from(err: E) -> Self {
        InOrderError::Service(err)
    }
}

impl<E: fmt::Debug> fmt::Debug for InOrderError<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            InOrderError::Service(e) => write!(f, "InOrderError::Service({:?})", e),
            InOrderError::Disconnected => write!(f, "InOrderError::Disconnected"),
        }
    }
}

impl<E: fmt::Display> fmt::Display for InOrderError<E> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            InOrderError::Service(e) => e.fmt(f),
            InOrderError::Disconnected => write!(f, "InOrder service disconnected"),
        }
    }
}

/// InOrder - The service will yield responses as they become available,
/// in the order that their originating requests were submitted to the service.
pub struct InOrder<S> {
    _t: PhantomData<S>,
}

impl<S> InOrder<S>
    where
        S: Service,
        S::Response: 'static,
        S::Future: 'static,
        S::Error: 'static,
{
    pub fn new() -> Self {
        Self { _t: PhantomData }
    }

    pub fn service(service: S) -> InOrderService<S> {
        InOrderService::new(service)
    }
}

impl<S> Default for InOrder<S>
    where
        S: Service,
        S::Response: 'static,
        S::Future: 'static,
        S::Error: 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<S> Transform<S> for InOrder<S>
    where
        S: Service,
        S::Response: 'static,
        S::Future: 'static,
        S::Error: 'static,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = InOrderError<S::Error>;
    type InitError = Infallible;
    type Transform = InOrderService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(InOrderService::new(service))
    }
}

#[pin_project]
pub struct InOrderService<S: Service> {
    #[pin]
    service: S,
    task: Rc<AtomicWaker>,
    acks: VecDeque<Record<S::Response, S::Error>>,
}

impl<S> InOrderService<S>
    where
        S: Service,
        S::Response: 'static,
        S::Future: 'static,
        S::Error: 'static,
{
    pub fn new<U>(service: U) -> Self
        where
            U: IntoService<S>,
    {
        Self {
            service: service.into_service(),
            acks: VecDeque::new(),
            task: Rc::new(AtomicWaker::new()),
        }
    }
}

impl<S> Service for InOrderService<S>
    where
        S: Service,
        S::Response: 'static,
        S::Future: 'static,
        S::Error: 'static,
{
    type Request = S::Request;
    type Response = S::Response;
    type Error = InOrderError<S::Error>;
    type Future = InOrderServiceResponse<S>;

    fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let mut this = self.project();
        // poll_ready could be called from different task
        this.task.register(ctx.waker());

        // check acks
        while !this.acks.is_empty() {
            let rec = this.acks.front_mut().unwrap();
            match Pin::new(&mut rec.rx).poll(ctx) {
                Poll::Ready(Ok(res)) => {
                    let rec = this.acks.pop_front().unwrap();
                    let _ = rec.tx.send(res);
                }
                Poll::Pending => break,
                Poll::Ready(Err(oneshot::Canceled)) => return Poll::Ready(Err(InOrderError::Disconnected)),
            }
        }

        // check nested service
        let () = ready!(this.service.poll_ready(ctx).map_err(InOrderError::Service))?;
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Self::Request) -> Self::Future {
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        self.acks.push_back(Record { rx: rx1, tx: tx2 });

        let task = self.task.clone();
        tokio_executor::current_thread::spawn(self.service.call(req).then(move |res| {
            task.wake();
            let _ = tx1.send(res);
            ready(())
        }));

        InOrderServiceResponse { rx: rx2 }
    }
}


#[doc(hidden)]
#[pin_project]
pub struct InOrderServiceResponse<S: Service> {
    #[pin]
    rx: oneshot::Receiver<Result<S::Response, S::Error>>,
}

impl<S: Service> Future for InOrderServiceResponse<S> {
    type Output = Result<S::Response, InOrderError<S::Error>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match ready!(this.rx.poll(cx)) {
            Ok(Ok(res)) => Poll::Ready(Ok(res)),
            Ok(Err(e)) => Poll::Ready(Err(e.into())),
            Err(oneshot::Canceled) => Poll::Ready(Err(InOrderError::Disconnected)),
        }
    }
}

#[cfg(test)]
mod tests {
    use futures::future::{lazy, Future, LocalBoxFuture};
    use futures::{stream::futures_unordered, channel::oneshot, Poll, Stream, StreamExt, TryFutureExt};

    use std::time::Duration;

    use super::*;
    use actix_service::blank::Blank;
    use actix_service::{Service, ServiceExt};
    use futures::stream::FuturesUnordered;

    struct Srv;

    impl Service for Srv {
        type Request = oneshot::Receiver<usize>;
        type Response = usize;
        type Error = ();
        type Future = LocalBoxFuture<'static, Result<usize, ()>>;

        fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, req: oneshot::Receiver<usize>) -> Self::Future {
            req.map_err(|_|()).boxed_local()
        }
    }

    #[pin_project]
    struct SrvPoll<S: Service> {
        #[pin]
        s: S,
    }

    impl<S: Service> Future for SrvPoll<S> {
        type Output = Result<(), ()>;

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
            let this = self.project();
            let _ = this.s.poll_ready(cx);
            Poll::Pending
        }
    }

    #[test]
    fn test_inorder() {
        let (tx1, rx1) = oneshot::channel();
        let (tx2, rx2) = oneshot::channel();
        let (tx3, rx3) = oneshot::channel();
        let (tx_stop, mut rx_stop) = oneshot::channel::<()>();

        let h = std::thread::spawn(move || {
            let rx1 = rx1;
            let rx2 = rx2;
            let rx3 = rx3;
            let tx_stop = tx_stop;
            let _ = actix_rt::System::new("test").block_on(async move {
                let mut srv = Blank::new().and_then(InOrderService::new(Srv));

                let res1 = srv.call(rx1);
                let res2 = srv.call(rx2);
                let res3 = srv.call(rx3);
                /*
                tokio_executor::current_thread::spawn(async move { SrvPoll { s: srv }.await; });

                futures::stream::iter(vec![res1, res2, res3].into_iter())
                    .collect::<FuturesUnordered<_>>()
                    .collect()
                    .and_then(move |res: Vec<_>| {
                        assert_eq!(res, vec![1, 2, 3]);
                        let _ = tx_stop.send(());
                        actix_rt::System::current().stop();
                        Ok(())
                    })*/
            });
        });

        let _ = tx3.send(3);
        std::thread::sleep(Duration::from_millis(50));
        let _ = tx2.send(2);
        let _ = tx1.send(1);

        let _ = rx_stop.close();
        let _ = h.join();
    }
}
