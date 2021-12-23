pub mod cmds;

use std::{
    any::TypeId,
    fmt::Debug,
    io::{self, IoSlice},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::{io::AsyncWriteExt, net::UnixStream};

use self::cmds::command::{Compute, Execute};
use crate::Error;

static SOCKET_GLOB: Lazy<String> = Lazy::new(|| format!("/tmp/{}/.mpvsocket*", whoami::username()));

static SOCKET_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\.mpvsocket([0-9]+)$").unwrap());

pub struct MpvSocket {
    path: PathBuf,
    socket: UnixStream,
}

impl MpvSocket {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub async fn lattest_cached() -> Result<Self, Error> {
        const INVALID_THREASHOLD: Duration = Duration::from_secs(30);

        static CURRENT: Lazy<Mutex<(PathBuf, Instant)>> = Lazy::new(|| {
            Mutex::new((
                PathBuf::new(),
                Instant::now()
                    .checked_sub(INVALID_THREASHOLD)
                    .unwrap_or_else(Instant::now),
            ))
        });

        let mut current = CURRENT.lock();
        if current.1.elapsed() >= INVALID_THREASHOLD {
            let Self { path, socket } = Self::lattest().await?;
            tracing::trace!("Cache hit {}", path.display());
            current.0 = path;
            current.1 = Instant::now();
            Ok(Self {
                socket,
                path: current.0.clone(),
            })
        } else {
            current.1 = Instant::now();
            match UnixStream::connect(&current.0).await {
                Ok(sock) => Ok(Self {
                    socket: sock,
                    path: current.0.clone(),
                }),
                Err(_) => {
                    let Self { path, socket } = Self::lattest().await?;
                    tracing::trace!("Cache miss. Opening {} instead", path.display());
                    current.0 = path.clone();
                    Ok(Self { socket, path })
                }
            }
        }
    }

    pub async fn new_path() -> Result<PathBuf, Error> {
        fn new_path(end: &str) -> PathBuf {
            let mut new_path = SOCKET_GLOB.clone();
            new_path.pop(); // remove '*'
            new_path.push_str(end);
            PathBuf::from(new_path)
        }

        let path = match Self::lattest().await {
            Ok(Self { path, .. }) => path,
            Err(Error::NoMpvInstance) => return Ok(new_path("0")),
            Err(e) => return Err(e),
        };

        let path = path.into_os_string();
        let path = path
            .to_str()
            .ok_or(Error::InvalidPath("path is not valid utf8"))?;

        let i = SOCKET_REGEX
            .find(path)
            .ok_or(Error::InvalidPath("path didn't contain a number"))?
            .as_str();

        Ok(new_path(i))
    }

    pub async fn new() -> Result<Self, Error> {
        let path = Self::new_path().await?;
        Ok(Self {
            socket: UnixStream::connect(&path).await?,
            path,
        })
    }

    pub async fn lattest() -> Result<Self, Error> {
        let mut available_sockets: Vec<_> = glob::glob(&*SOCKET_GLOB)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|x| x.to_str().map(|x| SOCKET_REGEX.is_match(x)) == Some(true))
            .collect();
        available_sockets.sort();
        for s in available_sockets.into_iter().rev() {
            if let Ok(sock) = UnixStream::connect(&s).await {
                return Ok(Self {
                    socket: sock,
                    path: s,
                });
            }
        }
        Err(Error::NoMpvInstance)
    }

    pub(crate) async fn mpv_do<S: Serialize + Debug, O: DeserializeOwned + Debug + 'static>(
        &mut self,
        cmd: S,
    ) -> Result<O, Error> {
        tracing::trace!(
            "trying to fetch a property of type: {}",
            std::any::type_name::<O>()
        );
        #[derive(Deserialize, Debug)]
        struct Payload<'e, O> {
            error: &'e str,
            data: Option<O>,
        }

        tracing::trace!("Checking if socket is writable");
        self.socket.writable().await?;
        tracing::trace!("Writing to the socket '{:?}'", cmd);
        let v = serde_json::to_vec(&serde_json::json!({ "command": cmd }))
            .expect("serialization to never fail");
        // TODO: check return of 0?
        self.writeln(&v).await?;

        let mut buf = Vec::with_capacity(1024);
        'readloop: loop {
            tracing::trace!("Waiting for the socket to become readable");
            self.socket.readable().await?;
            loop {
                tracing::trace!("Trying to read from socket");
                match self.socket.try_read_buf(&mut buf) {
                    Ok(_) => break 'readloop,
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        tracing::warn!("false positive read");
                    }
                    Err(e) => return Err(e.into()),
                };
            }
        }

        let start_i = match buf.iter().position(|b| *b != b'\0') {
            Some(i) => i,
            None => return Err(Error::Io(io::ErrorKind::UnexpectedEof.into())),
        };

        let payload = match buf[start_i..]
            .split(|&b| b == b'\n')
            .find_map(|b| serde_json::from_slice::<Payload<O>>(b).ok())
        {
            Some(payload) => payload,
            None => return Err(Error::Io(io::ErrorKind::InvalidData.into())),
        };

        match payload {
            Payload {
                error: "success",
                data: Some(data),
            } => Ok(data),

            Payload {
                error: "success",
                data: None,
            } => {
                if TypeId::of::<O>() == TypeId::of::<()>() {
                    Ok(unsafe { std::mem::transmute_copy(&()) })
                } else {
                    Err(Error::IpcError(format!(
                        "Call was successful, but there was no data field: {:?}",
                        std::str::from_utf8(&buf[start_i..])
                    )))
                }
            }

            Payload { error, .. } => Err(Error::IpcError(error.to_string())),
        }
    }

    pub async fn fire<S: AsRef<[u8]>>(&mut self, msg: S) -> io::Result<()> {
        self.writeln(msg.as_ref()).await?;
        Ok(())
    }

    async fn writeln(&mut self, b: &[u8]) -> io::Result<usize> {
        let io_slices = [IoSlice::new(b), IoSlice::new(b"\n")];
        self.socket.write_vectored(&io_slices).await
    }

    pub async fn compute<C, const N: usize>(&mut self, cmd: C) -> Result<C::Output, Error>
    where
        C: Compute<N>,
        C::Output: DeserializeOwned + Debug + 'static,
    {
        self.mpv_do(cmd.cmd().as_slice()).await
    }

    pub async fn execute<C, const N: usize>(&mut self, cmd: C) -> Result<(), Error>
    where
        C: Execute<N>,
    {
        self.mpv_do(cmd.cmd().as_slice()).await
    }
}
