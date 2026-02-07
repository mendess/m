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

use futures_util::{Stream, stream};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{
        UnixStream,
        unix::{OwnedReadHalf, OwnedWriteHalf},
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
    pub async fn new(
        name: &str,
        socket_path: &Path,
        auto_start: bool,
    ) -> io::Result<Self> {
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
        async fn exchange(
            reader: &mut BufReader<OwnedReadHalf>,
            writer: &mut BufWriter<OwnedWriteHalf>,
            message: &[u8],
        ) -> Result<String, io::Error> {
            writer.write_all(message).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
            let mut response = String::new();
            debug!("getting response message from daemon");
            reader.read_line(&mut response).await?;
            response.pop(); // trim newline
            Ok(response)
        }
        debug!(
            ?message,
            "sending message to daemon, type: {}",
            std::any::type_name::<M>()
        );
        let message = serde_json::to_vec(&message).unwrap();
        let response = match exchange(
            &mut self.reader,
            &mut self.writer,
            &message,
        )
        .await
        {
            Ok(r) => r,
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
                *self = Self::new(&self.name, &self.socket_path, false).await?;
                exchange(&mut self.reader, &mut self.writer, &message).await?
            }
            Err(e) => return Err(e),
        };
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
    pub async fn subscribe(
        mut self,
    ) -> Result<impl Stream<Item = io::Result<E>>, io::Error> {
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
