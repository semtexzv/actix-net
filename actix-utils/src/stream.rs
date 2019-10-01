use std::marker::PhantomData;
use std::rc::Rc;

use actix_service::{IntoService, NewService, Service};
use futures::channel::mpsc;
use futures::{ready, Future, Poll, Stream, TryFutureExt, TryFuture, FutureExt};
use std::pin::Pin;
use std::task::Context;

use pin_project::pin_project;
use futures::future::LocalBoxFuture;

type Request<T> = Result<<T as IntoStream>::Item, <T as IntoStream>::Error>;

pub trait IntoStream {
    type Item;
    type Error;
    type Stream: Stream<Item=Result<Self::Item, Self::Error>>;

    fn into_stream(self) -> Self::Stream;
}

impl<I, E, T> IntoStream for T
    where
        T: Stream<Item=Result<I, E>>,
{
    type Item = I;
    type Error = E;
    type Stream = T;

    fn into_stream(self) -> Self::Stream {
        self
    }
}

pub struct StreamService<S, T: NewService, E> {
    factory: Rc<T>,
    config: T::Config,
    _t: PhantomData<(S, E)>,
}

impl<S, T, E> Service for StreamService<S, T, E>
    where
        S: IntoStream + 'static,
        T: NewService<Request=Request<S>, Response=(), Error=E, InitError=E>,
        T::Future: 'static,
        T::Service: 'static,
        <T::Service as Service>::Future: 'static,
{
    type Request = S;
    type Response = ();
    type Error = E;
    type Future = LocalBoxFuture<'static, Result<(), E>>;

    fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: S) -> Self::Future {
        self.factory
            .new_service(&self.config)
            .and_then(move |srv| StreamDispatcher::new(req, srv))
            .boxed_local()
    }
}

#[pin_project]
pub struct StreamDispatcher<S, T>
    where
        S: IntoStream + 'static,
        T: Service<Request=Request<S>, Response=()> + 'static,
        T: 'static,
{
    #[pin]
    stream: S::Stream,
    #[pin]
    service: T,
    #[pin]
    err_rx: mpsc::UnboundedReceiver<T::Error>,
    #[pin]
    err_tx: mpsc::UnboundedSender<T::Error>,
}

impl<S, T> StreamDispatcher<S, T>
    where
        S: IntoStream,
        T: Service<Request=Request<S>, Response=()>,
        T::Future: 'static,
{
    pub fn new<F2>(stream: S, service: F2) -> Self
        where
            F2: IntoService<T>,
    {
        let (err_tx, err_rx) = mpsc::unbounded();
        StreamDispatcher {
            err_rx,
            err_tx,
            stream: stream.into_stream(),
            service: service.into_service(),
        }
    }
}


impl<S, T> Future for StreamDispatcher<S, T>
    where
        S: IntoStream + 'static,
        T: Service<Request=Request<S>, Response=()> + 'static,
{
    type Output = Result<(), T::Error>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.as_mut().project();
        if let Poll::Ready(Some(e)) = this.err_rx.poll_next(cx) {
            return Poll::Ready(Err(e));
        }

        loop {
            let mut this = self.as_mut().project();
            match this.service.as_mut().poll_ready(cx)? {
                Poll::Ready(_) => match this.stream.poll_next(cx) {
                    Poll::Ready(Some(Ok(item))) => {
                        let mut svc = unsafe { Pin::get_unchecked_mut(this.service) };
                        tokio_executor::current_thread::spawn(StreamDispatcherService {
                            fut: svc.call(Ok(item)),
                            stop: this.err_tx.clone(),
                        });
                    }
                    Poll::Ready(Some(Err(err))) => {
                        let mut svc = unsafe { Pin::get_unchecked_mut(this.service) };
                        tokio_executor::current_thread::spawn(StreamDispatcherService {
                            fut: svc.call(Err(err)),
                            stop: this.err_tx.clone(),
                        })
                    }
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(None) => return Poll::Ready(Ok(())),
                },
                Poll::Pending => return Poll::Pending,
            }
        }
    }



    /*
    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {

    }
    */
}

#[pin_project]
struct StreamDispatcherService<F: TryFuture> {
    #[pin]
    fut: F,
    stop: mpsc::UnboundedSender<F::Error>,
}


impl<F: TryFuture> Future for StreamDispatcherService<F> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        match ready!(this.fut.try_poll(cx)) {
            Ok(_) => Poll::Ready(()),
            Err(e) => {
                let _ = this.stop.unbounded_send(e);
                Poll::Ready(())
            }
        }
    }
}
