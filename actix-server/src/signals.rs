use std::io;

use actix_rt::spawn;
use futures::stream::{futures_unordered, FuturesUnordered, LocalBoxStream};
use futures::{
    Future, FutureExt, Poll, Stream, StreamExt, TryFutureExt, TryStream, TryStreamExt,
};

use crate::server::Server;
use actix_service::ServiceExt;
use futures::future::LocalBoxFuture;
use std::pin::Pin;
use std::task::Context;
use tokio_net::signal::unix::signal;

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
    streams: Vec<SigStream>,
}

type SigStream = LocalBoxStream<'static, Result<Signal, io::Error>>;

impl Signals {
    #[cfg(not(unix))]
    pub(crate) fn start(srv: Server) {
        spawn(async {
            let sig = tokio_net::signal::ctrl_c().await;
            srv.signal(sig);
        });
    }

    #[cfg(unix)]
    pub(crate) fn start(srv: Server) {
        use tokio_net::signal::unix;

        let mut sigs: Vec<_> = Vec::new();

        let mut sig_map = [
            (tokio_net::signal::unix::SignalKind::interrupt(), Signal::Int),
            (tokio_net::signal::unix::SignalKind::hangup(), Signal::Hup),
            (tokio_net::signal::unix::SignalKind::terminate(), Signal::Term),
            (tokio_net::signal::unix::SignalKind::quit(), Signal::Quit),
        ];

        for (kind, sig) in sig_map.into_iter() {
            let sig = sig.clone();
            let fut = signal(*kind).unwrap();
            sigs.push(fut.map(move |_| sig).boxed_local());
        }
        spawn(async move {
            let mut sigstream = futures::stream::select_all(sigs);
            if let Some(sig) = sigstream.next().await {
                srv.signal(sig);
            }
        })
    }
}
