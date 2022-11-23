//! This module defines the daemon process. This process listens to messages and responds to them
//! using a provided handler
//!
use std::{
    convert::Infallible,
    future::{pending, Future},
    io::{self, IoSlice},
    marker::PhantomData,
    path::Path,
};

use futures_util::future::OptionFuture;
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    signal::{
        unix::SignalKind,
        unix::{signal, Signal},
    },
    sync::oneshot,
};
use tracing::{debug, error, info};

use crate::Daemon;

/// A builder for a daemon process.
pub struct DaemonProcess<'s, M, R> {
    socket_path: &'s Path,
    shutdown: Option<oneshot::Receiver<()>>,
    _marker: PhantomData<(M, R)>,
}

impl<'s, M, R> DaemonProcess<'s, M, R> {
    pub async fn new(daemon: &'s Daemon<M, R>) -> DaemonProcess<'s, M, R> {
        Self {
            socket_path: daemon.socket_path().await,
            shutdown: None,
            _marker: PhantomData,
        }
    }
}

impl<'s, M, R> DaemonProcess<'s, M, R> {
    /// Provide a means of gracefully shuting down the daemon. Sending on this channel causes the
    /// daemon to terminate.
    pub fn with_shutdown(self, shutdown: oneshot::Receiver<()>) -> Self {
        Self {
            shutdown: Some(shutdown),
            ..self
        }
    }

    /// Start the daemon process with a handler. This functions returns error if initialization
    /// fails. If initialization does not fail this function never returns.
    pub async fn run<H, Fut>(mut self, handler: H) -> io::Result<Infallible>
    where
        M: DeserializeOwned + Serialize + Send + 'static,
        H: FnMut(M) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: DeserializeOwned + Serialize + Send,
    {
        let _ = tokio::fs::remove_file(&self.socket_path).await;
        let socket = UnixListener::bind(self.socket_path)?;
        debug!(socket_path = ?self.socket_path, "listening on");

        let mut term = signal(SignalKind::terminate()).ok();
        let mut ctrc = signal(SignalKind::interrupt()).ok();

        let shutdown = async {
            match self.shutdown.take() {
                Some(s) => s.await,
                None => pending().await,
            }
        };
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                Some(_) = recv_signal(term.as_mut()) => break,
                Some(_) = recv_signal(ctrc.as_mut()) => break,
                Ok(_) = &mut shutdown => break,
                accept = socket.accept() => match accept {
                    Ok((stream, addr)) => {
                        info!("got a new connection from {:?}", addr);
                        tokio::spawn(handle_task(stream, handler.clone()));
                    },
                    Err(e) => {
                        error!("failed to accept connection: {:?}", e);
                    }
                }
            }
        }
        let _ = tokio::fs::remove_file(&self.socket_path).await;
        info!("daemon exiting");
        std::process::exit(0);

        async fn recv_signal(fut: Option<&mut Signal>) -> Option<()> {
            OptionFuture::from(fut.map(Signal::recv)).await.flatten()
        }
    }
}

async fn handle_task<M, H, Fut>(mut stream: UnixStream, mut handler: H)
where
    H: FnMut(M) -> Fut,
    Fut: Future,
    M: DeserializeOwned,
    Fut::Output: Serialize,
{
    let (recv, mut send) = stream.split();
    let mut lines = BufReader::new(recv).lines();
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                debug!(?line, "received message");
                let response = match serde_json::from_str(&line) {
                    Ok(m) => {
                        let response = handler(m).await;
                        serde_json::to_string(&response).unwrap()
                    }
                    Err(e) => serde_json::to_string(&e.to_string()).unwrap(),
                };
                debug!(?response, "sending response");
                let vector = [IoSlice::new(response.as_bytes()), IoSlice::new(b"\n")];
                if let Err(e) = send.write_vectored(&vector).await {
                    error!("failed to respond to client: {:?}", e)
                }
            }
            Ok(None) => break,
            Err(e) => {
                error!(?e, "error reading line from client");
                break;
            }
        }
    }
}
