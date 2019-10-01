//! Framed dispatcher service and related utilities
use std::collections::VecDeque;
use std::{fmt, mem};

use actix_codec::{AsyncRead, AsyncWrite, Decoder, Encoder, Framed};
use actix_service::{IntoService, Service};
use futures::task::AtomicWaker;
use futures::channel::mpsc;
use futures::{Future, Poll, Sink, Stream};
use log::debug;

use crate::cell::Cell;
use std::task::Context;
use std::pin::Pin;

use pin_project::pin_project;

type Request<U> = <U as Decoder>::Item;
type Response<U> = <U as Encoder>::Item;

/// Framed transport errors
pub enum FramedTransportError<E, U: Encoder + Decoder> {
    Service(E),
    Encoder(<U as Encoder>::Error),
    Decoder(<U as Decoder>::Error),
}

impl<E, U: Encoder + Decoder> From<E> for FramedTransportError<E, U> {
    fn from(err: E) -> Self {
        FramedTransportError::Service(err)
    }
}

impl<E, U: Encoder + Decoder> fmt::Debug for FramedTransportError<E, U>
    where
        E: fmt::Debug,
        <U as Encoder>::Error: fmt::Debug,
        <U as Decoder>::Error: fmt::Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            FramedTransportError::Service(ref e) => {
                write!(fmt, "FramedTransportError::Service({:?})", e)
            }
            FramedTransportError::Encoder(ref e) => {
                write!(fmt, "FramedTransportError::Encoder({:?})", e)
            }
            FramedTransportError::Decoder(ref e) => {
                write!(fmt, "FramedTransportError::Encoder({:?})", e)
            }
        }
    }
}

impl<E, U: Encoder + Decoder> fmt::Display for FramedTransportError<E, U>
    where
        E: fmt::Display,
        <U as Encoder>::Error: fmt::Debug,
        <U as Decoder>::Error: fmt::Debug,
{
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            FramedTransportError::Service(ref e) => write!(fmt, "{}", e),
            FramedTransportError::Encoder(ref e) => write!(fmt, "{:?}", e),
            FramedTransportError::Decoder(ref e) => write!(fmt, "{:?}", e),
        }
    }
}

pub enum FramedMessage<T> {
    Message(T),
    Close,
}

/// FramedTransport - is a future that reads frames from Framed object
/// and pass then to the service.
#[pin_project]
pub struct FramedTransport<S, T, U>
    where
        S: Service<Request=Request<U>, Response=Response<U>>,
        S::Error: 'static,
        S::Future: 'static,
        T: AsyncRead + AsyncWrite,
        U: Encoder + Decoder,
        <U as Encoder>::Item: 'static,
        <U as Encoder>::Error: std::fmt::Debug,
{
    #[pin]
    service: S,
    state: TransportState<S, U>,
    #[pin]
    framed: Framed<T, U>,
    #[pin]
    rx: Option<mpsc::UnboundedReceiver<FramedMessage<<U as Encoder>::Item>>>,
    inner: Cell<FramedTransportInner<<U as Encoder>::Item, S::Error>>,
}

enum TransportState<S: Service, U: Encoder + Decoder> {
    Processing,
    Error(FramedTransportError<S::Error, U>),
    FramedError(FramedTransportError<S::Error, U>),
    FlushAndStop,
    Stopping,
}

struct FramedTransportInner<I, E> {
    buf: VecDeque<Result<I, E>>,
    task: AtomicWaker,
}

impl<S, T, U> FramedTransport<S, T, U>
    where
        S: Service<Request=Request<U>, Response=Response<U>>,
        S::Error: 'static,
        S::Future: 'static,
        T: AsyncRead + AsyncWrite,
        U: Decoder + Encoder,
        <U as Encoder>::Item: 'static,
        <U as Encoder>::Error: std::fmt::Debug,
{
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool {
        let mut this = self.project();
        loop {
            match this.service.as_mut().poll_ready(cx) {
                Poll::Ready(Ok(_)) => {
                    let item = match this.framed.as_mut().poll_next(cx) {
                        Poll::Ready(Some(Ok(el))) => el,
                        Poll::Ready(Some(Err(err))) => {
                            *this.state =
                                TransportState::FramedError(FramedTransportError::Decoder(err));
                            return true;
                        }
                        Poll::Pending => return false,
                        Poll::Ready(None) => {
                            *this.state = TransportState::Stopping;
                            return true;
                        }
                    };

                    let mut cell = this.inner.clone();
                    cell.get_mut().task.register(cx.waker());
                    let item = unsafe { Pin::get_unchecked_mut(this.service.as_mut()) }.call(item);
                    tokio_executor::current_thread::spawn(async move {
                        let item = item.await;
                        let inner = cell.get_mut();
                        inner.buf.push_back(item);
                        inner.task.wake();
                    });
                }
                Poll::Pending => return false,
                Poll::Ready(Err(err)) => {
                    *this.state = TransportState::Error(FramedTransportError::Service(err));
                    return true;
                }
            }
        }
    }
    /// write to framed object
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>) -> bool {
        let mut this = self.project();
        let inner = this.inner.get_mut();
        let mut rx_done = this.rx.is_none();
        let mut buf_empty = inner.buf.is_empty();
        loop {
            while !this.framed.is_write_buf_full() {
                if !buf_empty {
                    match inner.buf.pop_front().unwrap() {
                        Ok(msg) => {
                            if let Err(err) = unsafe { Pin::get_unchecked_mut(this.framed.as_mut()) }.force_send(msg) {
                                *this.state = TransportState::FramedError(
                                    FramedTransportError::Encoder(err),
                                );
                                return true;
                            }
                            buf_empty = inner.buf.is_empty();
                        }
                        Err(err) => {
                            *this.state =
                                TransportState::Error(FramedTransportError::Service(err));
                            return true;
                        }
                    }
                }

                if !rx_done && this.rx.is_some() {
                    match this.rx.as_mut().as_pin_mut().unwrap().poll_next(cx) {
                        Poll::Ready(Some(FramedMessage::Message(msg))) => {
                            if let Err(err) = unsafe { Pin::get_unchecked_mut(this.framed.as_mut()) }.force_send(msg) {
                                *this.state = TransportState::FramedError(
                                    FramedTransportError::Encoder(err),
                                );
                                return true;
                            }
                        }
                        Poll::Ready(Some(FramedMessage::Close)) => {
                            *this.state = TransportState::FlushAndStop;
                            return true;
                        }
                        Poll::Ready(None) => {
                            rx_done = true;
                            this.rx.set(None);
                        }
                        Poll::Pending => rx_done = true,
                        Poll::Ready(Some(_e)) => {
                            rx_done = true;
                            this.rx.set(None);
                        }
                    }
                }

                if rx_done && buf_empty {
                    break;
                }
            }

            if !this.framed.is_write_buf_empty() {
                match this.framed.as_mut().poll_flush(cx) {
                    Poll::Pending => break,
                    Poll::Ready(Err(err)) => {
                        debug!("Error sending data: {:?}", err);
                        *this.state =
                            TransportState::FramedError(FramedTransportError::Encoder(err));
                        return true;
                    }
                    Poll::Ready(Ok(_)) => (),
                }
            } else {
                break;
            }
        }

        false
    }
}


impl<S, T, U> FramedTransport<S, T, U>
    where
        S: Service<Request=Request<U>, Response=Response<U>>,
        S::Error: 'static,
        S::Future: 'static,
        T: AsyncRead + AsyncWrite,
        U: Decoder + Encoder,
        <U as Encoder>::Item: 'static,
        <U as Encoder>::Error: std::fmt::Debug,
{
    pub fn new<F: IntoService<S>>(framed: Framed<T, U>, service: F) -> Self {
        FramedTransport {
            framed,
            rx: None,
            service: service.into_service(),
            state: TransportState::Processing,
            inner: Cell::new(FramedTransportInner {
                buf: VecDeque::new(),
                task: AtomicWaker::new(),
            }),
        }
    }

    /// Get Sender
    pub fn set_receiver(
        mut self,
        rx: mpsc::UnboundedReceiver<FramedMessage<<U as Encoder>::Item>>,
    ) -> Self {
        self.rx = Some(rx);
        self
    }

    /// Get reference to a service wrapped by `FramedTransport` instance.
    pub fn get_ref(&self) -> &S {
        &self.service
    }

    /// Get mutable reference to a service wrapped by `FramedTransport`
    /// instance.
    pub fn get_mut(&mut self) -> &mut S {
        &mut self.service
    }

    /// Get reference to a framed instance wrapped by `FramedTransport`
    /// instance.
    pub fn get_framed(&self) -> &Framed<T, U> {
        &self.framed
    }

    /// Get mutable reference to a framed instance wrapped by `FramedTransport`
    /// instance.
    pub fn get_framed_mut(&mut self) -> &mut Framed<T, U> {
        &mut self.framed
    }
}

impl<S, T, U> Future for FramedTransport<S, T, U>
    where
        S: Service<Request=Request<U>, Response=Response<U>>,
        S::Error: 'static,
        S::Future: 'static,
        T: AsyncRead + AsyncWrite,
        U: Decoder + Encoder,
        <U as Encoder>::Item: 'static,
        <U as Encoder>::Error: std::fmt::Debug,
{
    type Output = Result<(), FramedTransportError<S::Error, U>>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match mem::replace(self.as_mut().project().state, TransportState::Processing) {
            TransportState::Processing => {
                if self.as_mut().poll_read(cx) || self.as_mut().poll_write(cx) {
                    self.as_mut().poll(cx)
                } else {
                    Poll::Pending
                }
            }
            TransportState::Error(err) => {
                let empty = self.as_mut().framed.is_write_buf_empty();
                if empty || (self.as_mut().poll_write(cx) || self.as_mut().project().framed.is_write_buf_empty())
                {
                    Poll::Ready(Err(err))
                } else {
                    let mut this = self.as_mut().project();
                    *this.state = TransportState::Error(err);
                    Poll::Pending
                }
            }
            TransportState::FlushAndStop => {
                let mut this = self.as_mut().project();
                if !this.framed.is_write_buf_empty() {
                    match this.framed.poll_flush(cx) {
                        Poll::Ready(Err(err)) => {
                            debug!("Error sending data: {:?}", err);
                            Poll::Ready(Ok(()))
                        }
                        Poll::Pending => Poll::Pending,
                        Poll::Ready(Ok(_)) => Poll::Ready(Ok(())),
                    }
                } else {
                    Poll::Ready(Ok(()))
                }
            }
            TransportState::FramedError(err) => Poll::Ready(Err(err)),
            TransportState::Stopping => Poll::Ready(Ok(())),
        }
    }
}
