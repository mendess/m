mod link;
mod process;

use std::{
    any::Any,
    fmt::Debug,
    io,
    path::{Path, PathBuf},
};

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
pub struct Daemon<M, R> {
    name: &'static str,
    channels: OnceCell<io::Result<Mutex<DaemonLink<M, R>>>>,
    socket_path: OnceCell<PathBuf>,
}

impl<M, R> Daemon<M, R> {
    pub const fn new(name: &'static str) -> Self {
        Daemon {
            name,
            channels: OnceCell::const_new(),
            socket_path: OnceCell::const_new(),
        }
    }

    async fn socket_path(&self) -> &Path {
        self.socket_path.get_or_init(|| async {
            let (path, e) = namespaced_tmp::async_impl::in_user_tmp(self.name).await;
            if let Some(e) = e {
                error!("failed to create tmp dir for {} daemon: {:?}", self.name, e);
            }
            path
        })
        .await
    }

    pub async fn build_daemon_process(&self) -> Option<DaemonProcess<M, R>> {
        if matches!(std::env::args().next(), Some(arg0) if arg0 == self.name) {
            Some(DaemonProcess::new(self).await)
        } else {
            None
        }
    }
}

impl<M, R> Daemon<M, R>
where
    M: Serialize + Any + Debug,
    R: DeserializeOwned,
{
    pub async fn exchange(&self, message: M) -> io::Result<R> {
        let channels = match self
            .channels
            .get_or_init(|| async {
                DaemonLink::new(self.name, self.socket_path().await)
                    .await
                    .map(Mutex::new)
            })
            .await
        {
            Ok(ch) => ch,
            Err(e) => return Err(e.kind().into()),
        };
        channels.lock().await.exchange(message).await
    }
}
