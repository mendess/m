mod link;
mod process;

use std::{
    any::Any,
    convert::Infallible,
    fmt::Debug,
    io,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use futures_util::Stream;
use link::DaemonLink;
use process::DaemonProcess;
use serde::{de::DeserializeOwned, Serialize};
use tokio::sync::{Mutex, OnceCell};
use tracing::error;

type ArcDaemonLink<M, R, E> = Arc<Mutex<DaemonLink<M, R, E>>>;

/// The idea of a daemon. Instances of this struct can be used to
/// - talk to an existing daemon
/// - "transform" a process into a daemon
///
/// Talking to a daemon implicitly starts a background process as a daemon
#[derive(Debug)]
pub struct Daemon<M, R, E = Infallible> {
    start_daemon: AtomicBool,
    name: &'static str,
    socket_namespace: Option<String>,
    channels: Mutex<Option<ArcDaemonLink<M, R, E>>>,
    socket_path: OnceCell<PathBuf>,
}

impl<M, R, E> Daemon<M, R, E> {
    pub const fn new(name: &'static str) -> Self {
        Daemon {
            start_daemon: AtomicBool::new(false),
            name,
            socket_namespace: None,
            channels: Mutex::const_new(None),
            socket_path: OnceCell::const_new(),
        }
    }

    async fn socket_path(&self) -> &Path {
        self.socket_path
            .get_or_init(|| async {
                let (path, e) = match &self.socket_namespace {
                    None => namespaced_tmp::async_impl::in_user_tmp(self.name).await,
                    Some(ns) => namespaced_tmp::async_impl::in_tmp(ns, self.name).await,
                };
                if let Some(e) = e {
                    error!("failed to create tmp dir for {} daemon: {:?}", self.name, e);
                }
                path
            })
            .await
    }

    pub fn overriding_socket_namespace_with(&self, new_namepsace: String) -> Self {
        Self {
            start_daemon: AtomicBool::new(self.start_daemon.load(Ordering::Relaxed)),
            name: self.name,
            socket_namespace: Some(new_namepsace),
            channels: Mutex::const_new(None),
            socket_path: OnceCell::const_new(),
        }
    }

    pub async fn wait_for_daemon_to_spawn(&self) {
        // reset the socket. If we are doing this we expect to not have a valid socket setup.
        *self.channels.lock().await = None;
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

    async fn channels(&self) -> io::Result<Arc<Mutex<DaemonLink<M, R, E>>>> {
        let mut channels = self.channels.lock().await;
        match &*channels {
            Some(ch) => Ok(ch.clone()),
            None => Ok(channels
                .insert(Arc::new(Mutex::new(
                    DaemonLink::new(
                        self.name,
                        self.socket_path().await,
                        self.start_daemon.load(Ordering::SeqCst),
                    )
                    .await?,
                )))
                .clone()),
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
        let mut channels = channels.lock().await;
        channels.exchange(message).await
    }
}

impl<M, R, E> Daemon<M, R, E>
where
    E: DeserializeOwned,
{
    #[tracing::instrument(skip_all)]
    pub async fn subscribe(&self) -> Result<impl Stream<Item = io::Result<E>>, io::Error> {
        tracing::debug!("getting channels");
        let ch = self.channels().await?;
        tracing::debug!("getting channels lock");
        let ch = ch.lock().await;
        tracing::debug!("cloning channels");
        let ch = ch.try_clone().await?;
        tracing::debug!("subscribing");
        ch.subscribe().await
    }
}
