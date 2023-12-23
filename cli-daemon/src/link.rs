use std::{
    any::Any,
    convert::Infallible,
    fmt::Debug,
    io,
    marker::PhantomData,
    os::unix::prelude::CommandExt,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use futures_util::{stream, Stream};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{
        unix::{OwnedReadHalf, OwnedWriteHalf},
        UnixStream,
    },
};
use tracing::debug;

#[derive(Debug)]
pub struct DaemonLink<M, R, E = Infallible> {
    reader: BufReader<OwnedReadHalf>,
    writer: BufWriter<OwnedWriteHalf>,
    socket_path: PathBuf,
    name: String,
    _marker: PhantomData<(M, R, E)>,
}

impl<M, R, E> DaemonLink<M, R, E> {
    /// Try to connect to the daemon.
    ///
    /// If the daemon isn't running and `auto_start` is `true`. It will attempt to start the daemon
    /// and connect to it.
    pub async fn new(name: &str, socket_path: &Path, auto_start: bool) -> io::Result<Self> {
        let try_connect = || async {
            debug!(?socket_path, "attempt to connect");
            UnixStream::connect(socket_path).await.map(|sock| {
                let (reader, writer) = sock.into_split();
                DaemonLink {
                    reader: BufReader::new(reader),
                    writer: BufWriter::new(writer),
                    socket_path: socket_path.into(),
                    name: name.into(),
                    _marker: PhantomData,
                }
            })
        };

        match try_connect().await {
            Ok(link) => return Ok(link),
            Err(e) if !auto_start => return Err(e),
            _ => {}
        }

        debug!(?name, ?socket_path, "starting the daemon");
        Command::new(std::env::current_exe()?).arg0(name).spawn()?;

        debug!(?name, ?socket_path, "establishing connection to daemon");
        for i in 1..=5 {
            tokio::time::sleep(Duration::from_millis(100 * i)).await;
            if let Ok(link) = try_connect().await {
                return Ok(link);
            }
        }
        try_connect().await
    }

    /// Try to clone this link and make a new independent one.
    pub async fn try_clone(&self) -> io::Result<Self> {
        Self::new(&self.name, &self.socket_path, false).await
    }
}

impl<M, R, E> DaemonLink<M, R, E>
where
    M: Serialize + Any + Debug,
    R: DeserializeOwned,
{
    pub async fn exchange(&mut self, message: M) -> Result<R, io::Error> {
        debug!(
            ?message,
            "sending message to daemon, type: {}",
            std::any::type_name::<M>()
        );
        let message = serde_json::to_vec(&message).unwrap();
        self.writer.write_all(&message).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        let mut response = String::new();
        debug!("getting response message from daemon");
        self.reader.read_line(&mut response).await?;
        response.pop(); // trim newline
        debug!(?response, "got");
        Ok(serde_json::from_str(&response)?)
    }
}

#[derive(Deserialize, Serialize)]
pub(crate) struct EventSubscription;

impl<M, R, E> DaemonLink<M, R, E>
where
    E: DeserializeOwned,
{
    pub async fn subscribe(mut self) -> Result<impl Stream<Item = io::Result<E>>, io::Error> {
        let message = serde_json::to_vec(&EventSubscription).unwrap();
        tracing::debug!(message = ?std::str::from_utf8(&message), "sending event subscription message");
        self.writer.write_all(&message).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(stream::try_unfold(
            (self, String::new()),
            move |(mut this, mut buf)| async {
                buf.clear();
                this.reader.read_line(&mut buf).await?;
                let ev = serde_json::from_str(&buf)?;
                Ok(Some((ev, (this, buf))))
            },
        ))
    }
}
