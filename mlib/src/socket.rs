pub mod cmds;
pub mod event;

use std::{
    any::TypeId,
    fmt::{Debug, Write},
    io::{self, IoSlice},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use futures_util::{stream, Stream, StreamExt};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
};

use self::cmds::command::{Compute, Execute, Property};
use crate::Error;
use arc_swap::ArcSwapOption;

static OVERRIDE: ArcSwapOption<PathBuf> = ArcSwapOption::const_empty();

pub fn override_lattest(id: usize) {
    let mut path = SOCKET_GLOB.clone();
    path.pop();
    let _ = write!(path, "{}", id);
    OVERRIDE.store(Some(Arc::new(PathBuf::from(path))))
}

static SOCKET_GLOB: Lazy<String> = Lazy::new(|| {
    let s = format!("/tmp/{}", whoami::username());
    let _ = std::fs::create_dir_all(&s);
    s + "/.mpvsocket*"
});

static SOCKET_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\.mpvsocket([0-9]+)$").unwrap());

#[derive(Debug)]
pub struct MpvSocket<S = UnixStream> {
    path: PathBuf,
    socket: S,
}

impl<S> PartialEq for MpvSocket<S> {
    fn eq(&self, other: &Self) -> bool {
        self.path.eq(&other.path)
    }
}

impl<S> Eq for MpvSocket<S> {}

impl<S> PartialOrd for MpvSocket<S> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.path.partial_cmp(&other.path)
    }
}

impl<S> Ord for MpvSocket<S> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.path.cmp(&other.path)
    }
}

impl<S> MpvSocket<S> {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

pub fn all() -> impl Stream<Item = MpvSocket<UnixStream>> {
    let mut paths = glob::glob(&*SOCKET_GLOB)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|x| x.to_str().map(|x| SOCKET_REGEX.is_match(x)) == Some(true))
        .collect::<Vec<_>>();
    paths.sort_unstable();
    paths.reverse();
    stream::iter(paths).filter_map(|path| async {
        UnixStream::connect(&path)
            .await
            .map(|socket| MpvSocket { socket, path })
            .ok()
    })
}

impl MpvSocket<()> {
    pub async fn connect(self) -> Result<MpvSocket<UnixStream>, (io::Error, Self)> {
        let socket = match UnixStream::connect(&self.path).await {
            Ok(s) => s,
            Err(e) => return Err((e, self)),
        };
        Ok(MpvSocket {
            socket,
            path: self.path,
        })
    }

    pub async fn new_unconnected() -> Result<MpvSocket<()>, Error> {
        fn new_path(end: &str) -> MpvSocket<()> {
            let mut new_path = SOCKET_GLOB.clone();
            new_path.pop(); // remove '*'
            new_path.push_str(end);
            MpvSocket {
                path: PathBuf::from(new_path),
                socket: (),
            }
        }

        let path = match MpvSocket::<UnixStream>::lattest().await {
            Ok(MpvSocket { path, .. }) => path,
            Err(Error::NoMpvInstance) => return Ok(new_path("0")),
            Err(e) => return Err(e),
        };

        let path = path.into_os_string();
        let path = path
            .to_str()
            .ok_or(Error::InvalidPath("path is not valid utf8"))?;

        let i = SOCKET_REGEX
            .captures(path)
            .ok_or(Error::InvalidPath("path didn't contain a number"))?
            .get(1)
            .unwrap()
            .as_str()
            .parse::<usize>()
            .unwrap()
            + 1;

        Ok(new_path(&i.to_string()))
    }
}

impl MpvSocket<UnixStream> {
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
            tracing::debug!("Cache hit {}", path.display());
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
                    tracing::debug!("Cache miss. Opening {} instead", path.display());
                    current.0 = path.clone();
                    Ok(Self { socket, path })
                }
            }
        }
    }

    pub async fn lattest() -> Result<Self, Error> {
        if let Some(path) = OVERRIDE.load_full() {
            Ok(Self {
                socket: UnixStream::connect(&*path).await?,
                path: path.to_path_buf(),
            })
        } else {
            let all = all();
            tokio::pin!(all);
            all.next().await.ok_or(Error::NoMpvInstance)
        }
    }

    pub(crate) async fn mpv_do<S: Serialize + Debug, O: DeserializeOwned + Debug + 'static>(
        &mut self,
        cmd: S,
    ) -> Result<O, Error> {
        tracing::debug!(
            "trying to fetch a property of type: {}",
            std::any::type_name::<O>()
        );
        #[derive(Deserialize, Debug)]
        struct Payload<'e, O> {
            error: &'e str,
            data: Option<O>,
        }

        tracing::debug!("Checking if socket is writable");
        self.socket.writable().await?;
        tracing::debug!("Writing to the socket '{:?}'", cmd);
        let v = serde_json::to_vec(&serde_json::json!({ "command": cmd }))
            .expect("serialization to never fail");
        // TODO: check return of 0?
        self.writeln(&v).await?;

        let mut buf = Vec::with_capacity(1024);
        'readloop: loop {
            tracing::debug!("Waiting for the socket to become readable");
            self.socket.readable().await?;
            loop {
                tracing::debug!("Trying to read from socket");
                match self.socket.try_read_buf(&mut buf) {
                    Ok(0) => break 'readloop,
                    Ok(_) => (),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if !buf.is_empty() {
                            break 'readloop;
                        }
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
            .find_map(|b| serde_json::from_slice::<Payload<'_, O>>(b).ok())
        {
            Some(payload) => payload,
            None => {
                tracing::debug!(
                    "could not deserialize {:?}",
                    std::str::from_utf8(&buf[start_i..])
                );
                return Err(Error::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    String::from_utf8_lossy(&buf[start_i..]),
                )));
            }
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
                if TypeId::of::<O>() == TypeId::of::<DevNull>() {
                    Ok(unsafe { std::mem::transmute_copy(&()) })
                } else {
                    Err(Error::IpcError(format!(
                        "Call was successful, but there was no data field: {:?}",
                        std::str::from_utf8(&buf[start_i..])
                    )))
                }
            }

            Payload { error, .. } => Err(Error::IpcError(format!(
                "{} :: {:?} => {}",
                error.to_string(),
                cmd,
                std::any::type_name::<O>()
            ))),
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
        self.mpv_do::<_, DevNull>(cmd.cmd().as_slice()).await?;
        Ok(())
    }

    pub async fn observe<P, F>(&mut self, mut f: F) -> Result<(), Error>
    where
        P: Property,
        F: FnMut(P::Output),
    {
        tracing::debug!(
            "trying to observe a property of type: {}",
            std::any::type_name::<P>()
        );
        tracing::debug!("Checking if socket is writable");
        self.socket.writable().await?;
        tracing::debug!(
            r#"Writing to the socket '["observe_property", {:?}]'"#,
            P::NAME
        );
        let v =
            serde_json::to_vec(&serde_json::json!({ "command": ["observe_property", 1, P::NAME] }))
                .expect("serialization to never fail");
        // TODO: check return of 0?
        self.writeln(&v).await?;

        let mut lines = BufReader::new(&mut self.socket).lines();
        if let Some(line) = lines.next_line().await? {
            #[derive(Deserialize, Debug)]
            struct Status<'s> {
                error: &'s str,
            }
            match serde_json::from_str::<Status<'_>>(&line) {
                Ok(Status { error: "success" }) => {}
                Ok(Status { error: _ }) => {
                    return Err(Error::IpcError(format!(
                        "failed to observe property {:?}: {:?}",
                        P::NAME,
                        line
                    )))
                }
                Err(e) => {
                    return Err(Error::IpcError(format!(
                        "failed to deserialize status from {:?}: {:?}",
                        line,
                        e
                    )))
                }
            }
        }
        while let Some(line) = lines.next_line().await? {
            #[derive(Deserialize, Debug)]
            struct Event<O> {
                data: O,
            }
            match serde_json::from_str::<Event<P::Output>>(&line) {
                Ok(Event { data }) => f(data),
                Err(e) => {
                    tracing::error!("failed to deserialize {:?}: {:?}", line, e)
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct DevNull {
    _m: std::marker::PhantomData<()>,
}

impl DevNull {
    const INST: Self = DevNull {
        _m: std::marker::PhantomData,
    };
}

impl<'de> Deserialize<'de> for DevNull {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct DVisitor;

        impl<'de> serde::de::Visitor<'de> for DVisitor {
            type Value = DevNull;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("this should not happen")
            }

            fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<Self::Value, E> {
                Ok(DevNull::INST)
            }

            fn visit_map<A>(self, mut m: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                while m.next_entry::<String, DevNull>()?.is_some() {}
                Ok(DevNull::INST)
            }
        }
        deserializer.deserialize_any(DVisitor)
    }
}
