use std::collections::VecDeque;
use std::marker::PhantomData;
use std::net::SocketAddr;

use actix_service::{NewService, Service};
use futures::future::{err, ok, Ready, LocalBoxFuture};
use futures::{Future, Poll, FutureExt};
use tokio_net::tcp::TcpStream;

use super::connect::{Address, Connect, Connection};
use super::error::ConnectError;
use std::pin::Pin;
use std::task::Context;

/// Tcp connector service factory
#[derive(Debug)]
pub struct TcpConnectorFactory<T>(PhantomData<T>);

impl<T> TcpConnectorFactory<T> {
    pub fn new() -> Self {
        TcpConnectorFactory(PhantomData)
    }

    /// Create tcp connector service
    pub fn service(&self) -> TcpConnector<T> {
        TcpConnector(PhantomData)
    }
}

impl<T> Default for TcpConnectorFactory<T> {
    fn default() -> Self {
        TcpConnectorFactory(PhantomData)
    }
}

impl<T> Clone for TcpConnectorFactory<T> {
    fn clone(&self) -> Self {
        TcpConnectorFactory(PhantomData)
    }
}

impl<T: Address + 'static > NewService for TcpConnectorFactory<T> {
    type Request = Connect<T>;
    type Response = Connection<T, TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = TcpConnector<T>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(self.service())
    }
}

/// Tcp connector service
#[derive(Default, Debug)]
pub struct TcpConnector<T>(PhantomData<T>);

impl<T> TcpConnector<T> {
    pub fn new() -> Self {
        TcpConnector(PhantomData)
    }
}

impl<T> Clone for TcpConnector<T> {
    fn clone(&self) -> Self {
        TcpConnector(PhantomData)
    }
}

impl<T: Address + 'static> Service for TcpConnector<T> {
    type Request = Connect<T>;
    type Response = Connection<T, TcpStream>;
    type Error = ConnectError;
    type Future = LocalBoxFuture<'static, Result<Connection<T, TcpStream>, ConnectError>>;

    fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }


    fn call(&mut self, req: Connect<T>) -> Self::Future {
        // TODO: logging
        let port = req.port();
        let Connect { req, addr, .. } = req;

        async move {
            let mut addr = if let Some(addr) = addr {
                addr
            } else {
                error!("TCP connector: got unresolved address");
                return Err(ConnectError::Unresolverd);
            };
            match addr {
                either::Either::Left(addr) => {
                    let stream = TcpStream::connect(addr).await?;
                    return Ok(Connection::new(stream, req));
                }
                either::Either::Right(ref mut addrs) => {
                    while let Some(addr) = addrs.pop_front() {
                        let stream = TcpStream::connect(addr).await;
                        if let Ok(stream) = stream {
                            return Ok(Connection::new(stream, req));
                        }
                    }
                }
            }

            Err(ConnectError::Unresolverd)
        }.boxed_local()
    }
}