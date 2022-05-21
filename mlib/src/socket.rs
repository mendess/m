pub mod cmds;
pub mod event;
pub mod local;
pub mod remote;

use std::{
    fmt::{Debug, Display},
    io,
    path::Path,
};

use futures_util::StreamExt;
use serde::de::DeserializeOwned;
use spark_protocol::{music::MpvMeta, Backend, Command};
use tokio::net::UnixStream;

use self::cmds::command::{Compute, Execute, Property};
use crate::Error;

#[derive(Debug)]
pub enum MpvSocket {
    Local(local::LocalMpvSocket<UnixStream>),
    Remote(remote::RemoteMpvSocket),
}

#[derive(Debug)]
pub struct UnconnectedMpvSocket {
    socket: local::LocalMpvSocket<()>,
    create_on_connect: bool,
}

impl UnconnectedMpvSocket {
    pub async fn connect(self) -> Result<MpvSocket, (io::Error, Self)> {
        match self.socket.connect().await {
            Ok(o) => {
                if self.create_on_connect {
                    let e = spark_protocol::client::send(Command::Backend(Backend::Music(
                        MpvMeta::CreatePlayer(o.index()),
                    )))
                    .await;
                    if let Err(e) = e {
                        tracing::error!(?e, "failed to communicate new socket");
                    }
                }
                Ok(MpvSocket::Local(o))
            }
            Err((e, s)) => Err((e, Self { socket: s, ..self })),
        }
    }

    pub fn create_on_connect(self) -> Self {
        Self {
            create_on_connect: true,
            ..self
        }
    }

    pub fn path(&self) -> &Path {
        self.socket.path()
    }

    //     pub(crate) async fn created_at(&self) -> io::Result<SystemTime> {
    //         tokio::fs::metadata(&self.path).await?.created()
    //     }
}

impl Display for UnconnectedMpvSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Unconnected({})", self.socket)
    }
}

pub async fn all() -> Vec<MpvSocket> {
    let mut local = local::all().collect::<Vec<_>>().await;
    let remote = match remote::all().await {
        Ok(s) => s.collect::<Vec<_>>().await,
        Err(e) => {
            tracing::debug!("Error fetching remote sockets, using locals: {:?}", e);
            return local.into_iter().map(MpvSocket::Local).collect();
        }
    };
    let this = whoami::hostname();
    if let Some(l) = local.iter().find(|l| {
        let index = l.index();
        !remote
            .iter()
            .any(|r| r.machine() == this && r.index() == index)
    }) {
        tracing::debug!(
            missing = %l,
            "Not all local sockets are in remote, using locals only"
        );
        return local.into_iter().map(MpvSocket::Local).collect();
    }
    remote
        .into_iter()
        .map(|r| {
            if r.machine() == this {
                if let Some(i) = local.iter().position(|l| l.index() == r.index()) {
                    return MpvSocket::Local(local.swap_remove(i));
                }
            }
            MpvSocket::Remote(r)
        })
        .collect()
}

impl MpvSocket {
    pub async fn current() -> Result<Self, Error> {
        let this = whoami::hostname();
        match remote::RemoteMpvSocket::current().await {
            Ok(s) => {
                if s.machine() == this {
                    Ok(Self::Local(
                        local::LocalMpvSocket::by_index(s.index()).await?,
                    ))
                } else {
                    Ok(Self::Remote(s))
                }
            }
            Err(e) => {
                tracing::warn!(?e, "failed to fetch remote current, using local");
                Ok(Self::Local(local::LocalMpvSocket::lattest().await?))
            }
        }
    }

    pub async fn new_unconnected() -> Result<UnconnectedMpvSocket, Error> {
        Ok(UnconnectedMpvSocket {
            socket: local::LocalMpvSocket::new_unconnected().await?,
            create_on_connect: false,
        })
    }

    pub async fn fire<S: AsRef<[u8]>>(&mut self, msg: S) -> Result<(), Error> {
        match self {
            Self::Local(l) => Ok(l.fire(msg).await?),
            Self::Remote(l) => l.fire(msg).await,
        }
    }

    pub async fn compute<C, const N: usize>(&mut self, cmd: C) -> Result<C::Output, Error>
    where
        C: Compute<N>,
        C::Output: DeserializeOwned + Debug + 'static,
    {
        match self {
            Self::Local(l) => l.compute(cmd).await,
            Self::Remote(l) => l.compute(cmd).await,
        }
    }

    pub async fn execute<C, const N: usize>(&mut self, cmd: C) -> Result<(), Error>
    where
        C: Execute<N>,
    {
        match self {
            Self::Local(l) => l.execute(cmd).await,
            Self::Remote(l) => l.execute(cmd).await,
        }
    }

    pub async fn observe<P, F>(&mut self, f: F) -> Result<(), Error>
    where
        P: Property,
        F: FnMut(P::Output),
    {
        match self {
            Self::Local(l) => l.observe::<P, F>(f).await,
            Self::Remote(l) => l.observe::<P, F>(f).await,
        }
    }

    pub async fn last(&mut self) -> Result<LastReference<'_>, Error> {
        Ok(LastReference {
            inner: match self {
                Self::Local(l) => LastRef::Local(l.last()),
                Self::Remote(l) => LastRef::Remote(l.last()),
            },
        })
    }
}

pub struct LastReference<'s> {
    inner: LastRef<'s>,
}

enum LastRef<'s> {
    Local(local::LastReference<'s>),
    Remote(remote::LastReference<'s>),
}

impl LastReference<'_> {
    pub async fn fetch(&mut self) -> Result<Option<usize>, Error> {
        match &mut self.inner {
            LastRef::Local(l) => l.fetch().await,
            LastRef::Remote(l) => l.fetch().await,
        }
    }

    pub async fn reset(&mut self) -> Result<(), Error> {
        match &mut self.inner {
            LastRef::Local(l) => l.reset().await,
            LastRef::Remote(l) => l.reset().await,
        }
    }

    pub async fn set(&mut self, n: usize) -> Result<(), Error> {
        match &mut self.inner {
            LastRef::Local(l) => l.set(n).await,
            LastRef::Remote(l) => l.set(n).await,
        }
    }
}

impl Display for MpvSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MpvSocket::Local(l) => {
                write!(f, "(localhost) {}", l)
            }
            MpvSocket::Remote(l) => {
                write!(f, "{}", l)
            }
        }
    }
}
