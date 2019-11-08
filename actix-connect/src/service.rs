use actix_service::{NewService, Service};
use futures::future::{ok, Ready};
use futures::{ready, Future, Poll};
use tokio_net::tcp::TcpStream;
use trust_dns_resolver::AsyncResolver;

use crate::connect::{Address, Connect, Connection};
use crate::connector::{TcpConnector, TcpConnectorFactory};
use crate::error::ConnectError;
use crate::resolver::{Resolver, ResolverFactory};

use pin_project::pin_project;
use std::pin::Pin;
use std::task::Context;

pub struct ConnectServiceFactory<T> {
    tcp: TcpConnectorFactory<T>,
    resolver: ResolverFactory<T>,
}

impl<T> ConnectServiceFactory<T> {
    /// Construct new ConnectService factory
    pub fn new() -> Self {
        ConnectServiceFactory {
            tcp: TcpConnectorFactory::default(),
            resolver: ResolverFactory::default(),
        }
    }

    /// Construct new connect service with custom dns resolver
    pub fn with_resolver(resolver: AsyncResolver) -> Self {
        ConnectServiceFactory {
            tcp: TcpConnectorFactory::default(),
            resolver: ResolverFactory::new(resolver),
        }
    }

    /// Construct new service
    pub fn service(&self) -> ConnectService<T> {
        ConnectService {
            tcp: self.tcp.service(),
            resolver: self.resolver.service(),
        }
    }

    /// Construct new tcp stream service
    pub fn tcp_service(&self) -> TcpConnectService<T> {
        TcpConnectService {
            tcp: self.tcp.service(),
            resolver: self.resolver.service(),
        }
    }
}

impl<T> Default for ConnectServiceFactory<T> {
    fn default() -> Self {
        ConnectServiceFactory {
            tcp: TcpConnectorFactory::default(),
            resolver: ResolverFactory::default(),
        }
    }
}

impl<T> Clone for ConnectServiceFactory<T> {
    fn clone(&self) -> Self {
        ConnectServiceFactory {
            tcp: self.tcp.clone(),
            resolver: self.resolver.clone(),
        }
    }
}

impl<T: Address> NewService for ConnectServiceFactory<T> {
    type Request = Connect<T>;
    type Response = Connection<T, TcpStream>;
    type Error = ConnectError;
    type Config = ();
    type Service = ConnectService<T>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(self.service())
    }
}

#[derive(Clone)]
pub struct ConnectService<T> {
    tcp: TcpConnector<T>,
    resolver: Resolver<T>,
}

impl<T: Address + 'static> Service for ConnectService<T> {
    type Request = Connect<T>;
    type Response = Connection<T, TcpStream>;
    type Error = ConnectError;
    type Future = impl Future<Output=Result<Connection<T, TcpStream>, ConnectError>>;

    fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }


    fn call(&mut self, req: Connect<T>) -> Self::Future {
        let mut fut = self.resolver.call(req);
        let mut tcp = self.tcp.clone();

        async move {
            let res = fut.await?;
            let res = tcp.call(res).await?;
            Ok(res)
        }
    }
}


#[derive(Clone)]
pub struct TcpConnectService<T> {
    tcp: TcpConnector<T>,
    resolver: Resolver<T>,
}

impl<T: Address + 'static> Service for TcpConnectService<T> {
    type Request = Connect<T>;
    type Response = TcpStream;
    type Error = ConnectError;
    type Future = impl Future<Output=Result<TcpStream, ConnectError>>;

    fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }


    fn call(&mut self, req: Connect<T>) -> Self::Future {
        let fut = self.resolver.call(req);
        let mut tcp = self.tcp.clone();
        async move {
            let fut = fut.await?;
            let fut = tcp.call(fut).await?;
            Ok(fut.into_parts().0)
        }
    }
}
