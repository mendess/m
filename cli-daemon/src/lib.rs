mod link;
mod process;

use std::{
    any::Any,
    convert::Infallible,
    fmt::Debug,
    io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use futures_util::Stream;
use link::DaemonLink;
use process::DaemonProcess;
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::{Mutex, OnceCell};
use tracing::error;

/// The idea of a daemon. Instances of this struct can be used to
/// - talk to an existing daemon
/// - "transform" a process into a daemon
///
/// Talking to a daemon implicitly starts a background process as a daemon
pub struct Daemon<M, R, E = Infallible> {
    start_daemon: AtomicBool,
    name: &'static str,
    channels: OnceCell<Mutex<DaemonLink<M, R, E>>>,
    socket_path: OnceCell<PathBuf>,
}

impl<M, R, E> Daemon<M, R, E> {
    pub const fn new(name: &'static str) -> Self {
        Daemon {
            start_daemon: AtomicBool::new(false),
            name,
            channels: OnceCell::const_new(),
            socket_path: OnceCell::const_new(),
        }
    }

    async fn socket_path(&self) -> &Path {
        self.socket_path
            .get_or_init(|| async {
                let (path, e) = namespaced_tmp::async_impl::in_user_tmp(self.name).await;
                if let Some(e) = e {
                    error!("failed to create tmp dir for {} daemon: {:?}", self.name, e);
                }
                path
            })
            .await
    }

    pub async fn wait_for_daemon_to_spawn(&self) {
        // TODO: make this smarter with ifnotify things
        loop {
            if self.channels().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    pub async fn build_daemon_process(&self) -> Option<DaemonProcess<M, R, E>> {
        if matches!(std::env::args().next(), Some(arg0) if arg0 == self.name) {
            Some(DaemonProcess::new(self).await)
        } else {
            self.start_daemon.store(true, Ordering::SeqCst);
            None
        }
    }

    async fn channels(&self) -> io::Result<&Mutex<DaemonLink<M, R, E>>> {
        match self
            .channels
            .get_or_try_init(|| async move {
                DaemonLink::new(
                    self.name,
                    self.socket_path().await,
                    self.start_daemon.load(Ordering::SeqCst),
                )
                .await
                .map(Mutex::new)
            })
            .await
        {
            Ok(ch) => Ok(ch),
            Err(e) => Err(e.kind().into()),
        }
    }
}

impl<M, R, E> Daemon<M, R, E>
where
    M: Serialize + Any + Debug,
    R: DeserializeOwned,
{
    pub async fn exchange(&self, message: M) -> io::Result<R> {
        let channels = self.channels().await?;
        channels.lock().await.exchange(message).await
    }
}

impl<M, R, E> Daemon<M, R, E>
where
    E: DeserializeOwned,
{
    pub async fn subscribe(&self) -> Result<impl Stream<Item = io::Result<E>>, io::Error> {
        self.channels()
            .await?
            .lock()
            .await
            .try_clone()
            .await?
            .subscribe()
            .await
    }
}
