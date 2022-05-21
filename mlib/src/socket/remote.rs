use std::{
    borrow::Cow,
    fmt::{Debug, Display},
    io,
};

use futures_util::{stream, Stream, StreamExt};
use serde::{de::DeserializeOwned, Deserialize};

use crate::Error;

use super::cmds::command::{Compute, Execute, Property};

use spark_protocol::{
    client::{Client, ClientBuilder},
    music::{self, LocalMetadata, MpvMeta, MusicCmdKind},
    Backend, Command, Local, ProtocolError, ProtocolMsg, Remote,
};

#[derive(Debug)]
pub struct RemoteMpvSocket {
    machine: String,
    index: u8,
    client: Client,
}

fn forward_deserialize<T: DeserializeOwned>(r: ProtocolMsg) -> Result<T, Error> {
    match r {
        ProtocolMsg::ForwardValue(r) => serde_json::from_value(r)
            .map_err(|e| Error::IpcError(format!("invalid forward {:?}", e))),
        _ => Err(Error::UnexpectedError(format!(
            "Unexpected response: {:?}",
            r
        ))),
    }
}

fn forward_error(e: ProtocolError) -> Error {
    match e {
        ProtocolError::ForwardedError(e) => Error::IpcError(e),
        e => Error::UnexpectedError(format!("{:#?}", e)),
    }
}

#[derive(Deserialize)]
struct RemoteSocketRef {
    hostname: String,
    player: u8,
}

pub(super) async fn all() -> Result<impl Stream<Item = RemoteMpvSocket>, Error> {
    spark_protocol::client::send(Command::Backend(Backend::Music(MpvMeta::ListPlayers)))
        .await?
        .ok_or_else(|| Error::IpcError("expected response, got EOF".into()))?
        .map_err(forward_error)
        .and_then(forward_deserialize::<Vec<RemoteSocketRef>>)
        .map(|v| {
            stream::iter(v.into_iter()).filter_map(
                |RemoteSocketRef { hostname, player }| async move {
                    match RemoteMpvSocket::new(hostname, player).await {
                        Ok(s) => Some(s),
                        Err((machine, index, e)) => {
                            tracing::debug!(
                                %machine,
                                %index,
                                ?e,
                                "Error constructing remote socket",
                            );
                            None
                        }
                    }
                },
            )
        })
}

impl RemoteMpvSocket {
    pub async fn current() -> Result<Self, Error> {
        tracing::info!("creating client");
        let mut client = ClientBuilder::new().build().await?;
        tracing::info!("sending command");
        let response = client
            .send(Command::Backend(Backend::Music(
                music::MpvMeta::GetCurrentPlayer,
            )))
            .await?
            .ok_or_else(|| Error::IpcError("expected response, got EOF".into()))?
            .map_err(forward_error)?;
        tracing::info!("command sent");
        match response {
            ProtocolMsg::Unit => Err(Error::UnexpectedError("expected a value, got unit".into())),
            ProtocolMsg::ForwardValue(v) => {
                let RemoteSocketRef { hostname, player } = serde_json::from_value(v)?;
                Ok(Self {
                    machine: hostname,
                    index: player,
                    client,
                })
            }
        }
    }

    pub async fn new(machine: String, index: u8) -> Result<Self, (String, u8, io::Error)> {
        let client = match ClientBuilder::new().build().await {
            Ok(c) => c,
            Err(e) => return Err((machine, index, e)),
        };
        Ok(Self {
            machine,
            index,
            client,
        })
    }

    async fn mpv_do(&mut self, command: music::MusicCmdKind<'_>) -> Result<ProtocolMsg, Error> {
        self.client
            .send(Command::Remote(Remote {
                machine: Cow::Borrowed(&self.machine),
                command: Local::Music(music::MusicCmd {
                    index: self.index,
                    command,
                }),
            }))
            .await?
            .ok_or_else(|| Error::IpcError("expected response, got EOF".into()))?
            .map_err(forward_error)
    }

    pub async fn fire<S: AsRef<[u8]>>(&mut self, msg: S) -> Result<(), Error> {
        self.mpv_do(MusicCmdKind::Fire(msg.as_ref().into()))
            .await
            .map(|_| ())
    }

    pub async fn compute<C, const N: usize>(&mut self, cmd: C) -> Result<C::Output, Error>
    where
        C: Compute<N>,
        C::Output: DeserializeOwned + Debug + 'static,
    {
        self.mpv_do(MusicCmdKind::Compute(
            serde_json::to_value(cmd.cmd().as_slice()).expect("serialization to never fail"),
        ))
        .await
        .and_then(forward_deserialize)
    }

    pub async fn execute<C, const N: usize>(&mut self, cmd: C) -> Result<(), Error>
    where
        C: Execute<N>,
    {
        self.mpv_do(MusicCmdKind::Execute(Cow::Owned(
            serde_json::to_vec(cmd.cmd().as_slice()).expect("serialization to never fail"),
        )))
        .await
        .map(|_| ())
    }

    pub async fn observe<P, F>(&mut self, _f: F) -> Result<(), Error>
    where
        P: Property,
        F: FnMut(P::Output),
    {
        todo!("remote observing not implemented yet")
    }

    pub fn machine(&self) -> &str {
        &self.machine
    }

    pub fn index(&self) -> u8 {
        self.index
    }

    pub fn last(&mut self) -> LastReference<'_> {
        LastReference(self)
    }
}

pub struct LastReference<'s>(&'s mut RemoteMpvSocket);

impl LastReference<'_> {
    #[inline]
    async fn mpv_do(&mut self, command: LocalMetadata) -> Result<ProtocolMsg, Error> {
        self.0.mpv_do(MusicCmdKind::Meta(command)).await
    }

    pub async fn fetch(&mut self) -> Result<Option<usize>, Error> {
        self.mpv_do(LocalMetadata::LastFetch)
            .await
            .and_then(forward_deserialize)
    }

    pub async fn reset(&mut self) -> Result<(), Error> {
        self.mpv_do(LocalMetadata::LastReset).await.map(|_| ())
    }

    pub async fn set(&mut self, n: usize) -> Result<(), Error> {
        self.mpv_do(LocalMetadata::LastSet(n)).await.map(|_| ())
    }
}

pub async fn create_new_player(n: u8) -> Result<(), Error> {
    spark_protocol::client::send(Command::Backend(Backend::Music(MpvMeta::CreatePlayer(n))))
        .await?
        .ok_or_else(|| Error::IpcError("expected response, got EOF".into()))?
        .map_err(forward_error)
        .map(|_| ())
}

pub async fn delete_player(n: u8) -> Result<(), Error> {
    spark_protocol::client::send(Command::Backend(Backend::Music(MpvMeta::DeletePlayer(n))))
        .await?
        .ok_or_else(|| Error::IpcError("expected response, got EOF".into()))?
        .map_err(forward_error)
        .map(|_| ())
}

pub async fn set_default_player(n: u8) -> Result<(), Error> {
    spark_protocol::client::send(Command::Backend(Backend::Music(MpvMeta::SetCurrentPlayer(
        n,
    ))))
    .await?
    .ok_or_else(|| Error::IpcError("expected response, got EOF".into()))?
    .map_err(forward_error)
    .map(|_| ())
}

impl Display for RemoteMpvSocket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}) index: {}", self.machine, self.index)
    }
}
