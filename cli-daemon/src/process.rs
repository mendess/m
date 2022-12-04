//! This module defines the daemon process. This process listens to messages and responds to them
//! using a provided handler
//!
use std::{
    convert::Infallible,
    future::{pending, Future},
    io,
    marker::PhantomData,
    path::Path,
};

use futures_util::{future::OptionFuture, stream, Stream, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{unix::WriteHalf, UnixListener, UnixStream},
    signal::{
        unix::SignalKind,
        unix::{signal, Signal},
    },
    sync::oneshot,
};
use tracing::{debug, error, info};

use crate::{link::EventSubscription, Daemon};

/// A builder for a daemon process.
pub struct DaemonProcess<'s, M, R, E = Infallible> {
    socket_path: &'s Path,
    shutdown: Option<oneshot::Receiver<()>>,
    _marker: PhantomData<(M, R, E)>,
}

impl<'s, M, R, E> DaemonProcess<'s, M, R, E> {
    pub async fn new(daemon: &'s Daemon<M, R, E>) -> DaemonProcess<'s, M, R, E> {
        Self {
            socket_path: daemon.socket_path().await,
            shutdown: None,
            _marker: PhantomData,
        }
    }
}

impl<'s, M, R, E> DaemonProcess<'s, M, R, E> {
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
    pub async fn run<H, Fut>(self, handler: H) -> io::Result<Infallible>
    where
        M: DeserializeOwned + Serialize + Send + 'static,
        H: FnMut(M) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = R> + Send + 'static,
        R: DeserializeOwned + Serialize + Send + Sync,
    {
        let DaemonProcess {
            socket_path,
            shutdown,
            ..
        } = self;
        DaemonProcess {
            socket_path,
            shutdown,
            _marker: PhantomData::<(M, R, ())>,
        }
        .run_with_events(handler, || async { stream::iter([]) })
        .await
    }
}

impl<'s, M, R, E> DaemonProcess<'s, M, R, E>
where
    E: Serialize + Send + Sync + 'static,
    M: DeserializeOwned + Serialize + Send + 'static,
    R: DeserializeOwned + Serialize + Send + Sync,
{
    /// Start the daemon process with a handler. This functions returns error if initialization
    /// fails. If initialization does not fail this function never returns.
    pub async fn run_with_events<H, Fut, EH, EHFut>(
        mut self,
        handler: H,
        events: EH,
    ) -> io::Result<Infallible>
    where
        EH: FnOnce() -> EHFut + Clone + Send + 'static,
        EHFut: Future + Send + 'static,
        EHFut::Output: Stream<Item = E> + Send + 'static,
        H: FnMut(M) -> Fut + Clone + Send + 'static,
        Fut: Future<Output = R> + Send + 'static,
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
                        tokio::spawn(handle_task(stream, handler.clone(), events.clone()));
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

async fn handle_task<M, H, Fut, E, EFut>(mut stream: UnixStream, mut handler: H, events: E)
where
    E: FnOnce() -> EFut,
    EFut: Future,
    EFut::Output: Stream,
    <EFut::Output as Stream>::Item: Serialize,
    H: FnMut(M) -> Fut,
    Fut: Future,
    M: DeserializeOwned,
    Fut::Output: Serialize,
{
    let (recv, send) = stream.split();
    let mut lines = BufReader::new(recv).lines();
    let mut send = BufWriter::new(send);
    loop {
        match lines.next_line().await {
            Ok(Some(line)) => {
                debug!(?line, "received message");
                match serde_json::from_str(&line) {
                    Ok(EventSubscription) => {
                        let stream = events().await;
                        tokio::pin!(stream);
                        while let Some(e) = stream.next().await {
                            if let Err(e) = send_msg(&mut send, &e).await {
                                error!(?e, "failed to send event to client");
                                break;
                            }
                        }
                        break;
                    }
                    Err(_) => {
                        let e = match serde_json::from_str(&line) {
                            Ok(m) => send_msg(&mut send, &handler(m).await).await,
                            Err(e) => send_msg(&mut send, &e.to_string()).await,
                        };
                        if let Err(e) = e {
                            error!(?e, "failed to respond to client");
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => {
                error!(?e, "error reading line from client");
                break;
            }
        }
    }

    async fn send_msg<M: Serialize>(sink: &mut BufWriter<WriteHalf<'_>>, m: &M) -> io::Result<()> {
        let response = serde_json::to_vec(m).unwrap();
        debug!(?response, "sending response");
        sink.write_all(&response).await?;
        sink.write_all(b"\n").await?;
        sink.flush().await?;
        Ok(())
    }
}
