use std::{
    any::Any,
    fmt::Debug,
    io::{self, IoSlice},
    marker::PhantomData,
    os::unix::prelude::CommandExt,
    path::Path,
    process::Command,
    time::Duration,
};

use serde::{de::DeserializeOwned, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{
        unix::{OwnedReadHalf, OwnedWriteHalf},
        UnixStream,
    },
};
use tracing::debug;

pub struct DaemonLink<M, R> {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    _marker: PhantomData<(M, R)>,
}

impl<M, R> DaemonLink<M, R> {
    pub async fn new(name: &str, socket_path: &Path) -> io::Result<Self> {
        let try_connect = || async {
            debug!(?socket_path, "attempt to connect");
            UnixStream::connect(socket_path).await.map(|sock| {
                let (reader, writer) = sock.into_split();
                DaemonLink {
                    reader: BufReader::new(reader),
                    writer,
                    _marker: PhantomData,
                }
            })
        };

        if let Ok(link) = try_connect().await {
            return Ok(link);
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
}

impl<M, R> DaemonLink<M, R>
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
        let vector = [IoSlice::new(&message), IoSlice::new(b"\n")];
        let len = self.writer.write_vectored(&vector).await?;
        let expected_len = vector[0].len() + vector[1].len();
        if len < expected_len {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("message to daemon process was truncated: {len} < {expected_len}"),
            ));
        }

        let mut response = String::new();
        debug!("getting response message from daemon");
        self.reader.read_line(&mut response).await?;
        response.pop(); // trim newline
        debug!(?response, "got");
        Ok(serde_json::from_str(&response)?)
    }
}
