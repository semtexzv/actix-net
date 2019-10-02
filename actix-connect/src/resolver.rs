use std::marker::PhantomData;
use std::net::SocketAddr;

use actix_service::{NewService, Service};
use futures::future::{ok, Either, Ready};
use futures::{Future, Poll};
use trust_dns_resolver::lookup_ip::LookupIpFuture;
use trust_dns_resolver::{AsyncResolver, Background};

use crate::connect::{Address, Connect};
use crate::error::ConnectError;
use crate::get_default_resolver;
use std::pin::Pin;
use std::task::Context;

/// DNS Resolver Service factory
pub struct ResolverFactory<T> {
    resolver: Option<AsyncResolver>,
    _t: PhantomData<T>,
}

impl<T> ResolverFactory<T> {
    /// Create new resolver instance with custom configuration and options.
    pub fn new(resolver: AsyncResolver) -> Self {
        ResolverFactory {
            resolver: Some(resolver),
            _t: PhantomData,
        }
    }

    pub fn service(&self) -> Resolver<T> {
        Resolver {
            resolver: self.resolver.clone(),
            _t: PhantomData,
        }
    }
}

impl<T> Default for ResolverFactory<T> {
    fn default() -> Self {
        ResolverFactory {
            resolver: None,
            _t: PhantomData,
        }
    }
}

impl<T> Clone for ResolverFactory<T> {
    fn clone(&self) -> Self {
        ResolverFactory {
            resolver: self.resolver.clone(),
            _t: PhantomData,
        }
    }
}

impl<T: Address> NewService for ResolverFactory<T> {
    type Request = Connect<T>;
    type Response = Connect<T>;
    type Error = ConnectError;
    type Config = ();
    type Service = Resolver<T>;
    type InitError = ();
    type Future = Ready<Result<Self::Service, Self::InitError>>;

    fn new_service(&self, _: &()) -> Self::Future {
        ok(self.service())
    }
}

/// DNS Resolver Service
pub struct Resolver<T> {
    resolver: Option<AsyncResolver>,
    _t: PhantomData<T>,
}

impl<T> Resolver<T> {
    /// Create new resolver instance with custom configuration and options.
    pub fn new(resolver: AsyncResolver) -> Self {
        Resolver {
            resolver: Some(resolver),
            _t: PhantomData,
        }
    }
}

impl<T> Default for Resolver<T> {
    fn default() -> Self {
        Resolver {
            resolver: None,
            _t: PhantomData,
        }
    }
}

impl<T> Clone for Resolver<T> {
    fn clone(&self) -> Self {
        Resolver {
            resolver: self.resolver.clone(),
            _t: PhantomData,
        }
    }
}

impl<T: Address> Service for Resolver<T> {
    type Request = Connect<T>;
    type Response = Connect<T>;
    type Error = ConnectError;
    type Future = impl Future<Output=Result<Connect<T>, ConnectError>>;

    fn poll_ready(self: Pin<&mut Self>, ctx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }


    fn call(&mut self, mut req: Connect<T>) -> Self::Future {
        if self.resolver.is_none() {
            self.resolver = Some(get_default_resolver());
        }
        async move {
            if req.addr.is_some() {
                return Ok(req);
            } else if let Ok(ip) = req.host().parse() {
                req.addr = Some(either::Either::Left(SocketAddr::new(ip, req.port())));
                return Ok(req);
            } else {
                trace!("DNS resolver: resolving host {:?}", req.host());

                unimplemented!("Waiting for trust-dns-resolver to migrate to std::future")
            }
            Err(ConnectError::Unresolverd)
        }

    }
}
