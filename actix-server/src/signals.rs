use std::io;

use actix_rt::spawn;
use futures::stream::futures_unordered;
use futures::{Future, Poll, Stream, TryStream};

use crate::server::Server;
use std::pin::Pin;
use std::task::Context;
use futures::future::LocalBoxFuture;

/// Different types of process signals
#[derive(PartialEq, Clone, Copy, Debug)]
pub(crate) enum Signal {
    /// SIGHUP
    Hup,
    /// SIGINT
    Int,
    /// SIGTERM
    Term,
    /// SIGQUIT
    Quit,
}

pub(crate) struct Signals {
    srv: Server,
    #[cfg(not(unix))]
    stream: SigStream,
    #[cfg(unix)]
    streams: Vec<SigStream>,
}

type SigStream = Box<dyn TryStream<Ok = Signal, Error = io::Error>>;

impl Signals {
    pub(crate) fn start(srv: Server) {
        let fut = {
            #[cfg(not(unix))]
            {
                tokio_net::signal::ctrl_c()
                    .map_err(|_| ())
                    .and_then(move |stream| Signals {
                        srv,
                        stream: Box::new(stream.map(|_| Signal::Int)),
                    })
            }

            #[cfg(unix)]
            {
                use tokio_net::signal::unix;

                let mut sigs: Vec<LocalBoxFuture<'static, Result<SigStream,io::Error>>> = 
                    Vec::new();
                sigs.push(Box::new(
                    tokio_net::signal::unix::Signal::new(tokio_net::signal::unix::libc::SIGINT).map(|stream| {
                        let s: SigStream = Box::new(stream.map(|_| Signal::Int));
                        s
                    }),
                ));
                sigs.push(Box::new(
                    tokio_net::signal::unix::Signal::new(tokio_net::signal::unix::libc::SIGHUP).map(
                        |stream: unix::Signal| {
                            let s: SigStream = Box::new(stream.map(|_| Signal::Hup));
                            s
                        },
                    ),
                ));
                sigs.push(Box::new(
                    tokio_net::signal::unix::Signal::new(tokio_net::signal::unix::libc::SIGTERM).map(
                        |stream| {
                            let s: SigStream = Box::new(stream.map(|_| Signal::Term));
                            s
                        },
                    ),
                ));
                sigs.push(Box::new(
                    tokio_net::signal::unix::Signal::new(tokio_net::signal::unix::libc::SIGQUIT).map(
                        |stream| {
                            let s: SigStream = Box::new(stream.map(|_| Signal::Quit));
                            s
                        },
                    ),
                ));
                futures_unordered(sigs)
                    .collect()
                    .map_err(|_| ())
                    .and_then(move |streams| Signals { srv, streams })
            }
        };
        spawn(fut);
    }
}

impl Future for Signals {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unimplemented!()
    }

    /*
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        #[cfg(not(unix))]
        loop {
            match self.stream.poll() {
                Ok(Async::Ready(None)) | Err(_) => return Ok(Async::Ready(())),
                Ok(Async::Ready(Some(sig))) => self.srv.signal(sig),
                Ok(Async::NotReady) => return Ok(Async::NotReady),
            }
        }
        #[cfg(unix)]
        {
            for s in &mut self.streams {
                loop {
                    match s.poll() {
                        Ok(Async::Ready(None)) | Err(_) => return Ok(Async::Ready(())),
                        Ok(Async::NotReady) => break,
                        Ok(Async::Ready(Some(sig))) => self.srv.signal(sig),
                    }
                }
            }
            Ok(Async::NotReady)
        }
    }
    */
}
