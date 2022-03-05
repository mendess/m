use std::{
    any::TypeId,
    fmt::{Debug, Display, Write},
    io::{self, IoSlice},
    os::unix::prelude::OsStrExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant, SystemTime},
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

use super::cmds::command::{Compute, Execute, Property};
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
    let (path, e) = namespaced_tmp::blocking::in_user_tmp(".mpvsocket*");
    if let Some(e) = e {
        tracing::error!("failed to create socket dir: {:?}", e);
    }
    path.display().to_string()
});

static SOCKET_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\.mpvsocket([0-9]+)$").unwrap());

#[derive(Debug)]
pub struct LocalMpvSocket<S = UnixStream> {
    path: PathBuf,
    socket: S,
}

impl<S> PartialEq for LocalMpvSocket<S> {
    fn eq(&self, other: &Self) -> bool {
        self.path.eq(&other.path)
    }
}

impl<S> Eq for LocalMpvSocket<S> {}

impl<S> PartialOrd for LocalMpvSocket<S> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.path.partial_cmp(&other.path)
    }
}

impl<S> Ord for LocalMpvSocket<S> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.path.cmp(&other.path)
    }
}

impl<S> LocalMpvSocket<S> {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn index(&self) -> u8 {
        SOCKET_REGEX
            .captures(std::str::from_utf8(self.path.as_os_str().as_bytes()).unwrap())
            .expect("a conforming path")[1]
            .parse()
            .expect("a conforming path")
    }

    async fn created_at(&self) -> io::Result<SystemTime> {
        tokio::fs::metadata(&self.path).await?.created()
    }
}

pub fn all() -> impl Stream<Item = LocalMpvSocket<UnixStream>> {
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
            .map(|socket| LocalMpvSocket { socket, path })
            .ok()
    })
}

impl LocalMpvSocket<()> {
    pub async fn connect(self) -> Result<LocalMpvSocket<UnixStream>, (io::Error, Self)> {
        let socket = match UnixStream::connect(&self.path).await {
            Ok(s) => s,
            Err(e) => return Err((e, self)),
        };
        Ok(LocalMpvSocket {
            socket,
            path: self.path,
        })
    }

    pub async fn new_unconnected() -> Result<LocalMpvSocket<()>, Error> {
        fn new_path(end: &str) -> LocalMpvSocket<()> {
            let mut new_path = SOCKET_GLOB.clone();
            new_path.pop(); // remove '*'
            new_path.push_str(end);
            LocalMpvSocket {
                path: PathBuf::from(new_path),
                socket: (),
            }
        }

        let path = match LocalMpvSocket::<UnixStream>::lattest().await {
            Ok(LocalMpvSocket { path, .. }) => path,
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

impl LocalMpvSocket<UnixStream> {
    pub async fn lattest_cached() -> Result<Self, Error> {
        const INVALID_THRESHOLD: Duration = Duration::from_secs(30);

        static CURRENT: Lazy<Mutex<(PathBuf, Instant)>> = Lazy::new(|| {
            Mutex::new((
                PathBuf::new(),
                Instant::now()
                    .checked_sub(INVALID_THRESHOLD)
                    .unwrap_or_else(Instant::now),
            ))
        });

        let mut current = CURRENT.lock();
        if current.1.elapsed() >= INVALID_THRESHOLD {
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
                    current.0.clone_from(&path);
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

    pub async fn by_index(n: u8) -> Result<Self, Error> {
        let path = SOCKET_GLOB.replace('*', &n.to_string());
        Ok(Self {
            socket: match UnixStream::connect(&path).await {
                Ok(s) => s,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(Error::NoMpvInstance),
                Err(e) => return Err(e.into()),
            },
            path: path.into(),
        })
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
                tracing::debug!("Trying to read from socket...");
                match self.socket.try_read_buf(&mut buf) {
                    Ok(0) => break 'readloop,
                    Ok(_) => (),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        if !buf.is_empty() {
                            break 'readloop;
                        }
                        tracing::warn!("false positive read");
                    }
                    Err(e) => {
                        tracing::error!(?e, buf_so_far=?buf, "error reading");
                        return Err(e.into());
                    }
                };
            }
        }

        tracing::debug!("finding the end of the buffer");
        let start_i = match buf.iter().position(|b| *b != b'\0') {
            Some(i) => i,
            None => {
                tracing::debug!(buf_len = buf.len(), "buffer did not contain a null byte");
                return Err(Error::Io(io::ErrorKind::UnexpectedEof.into()));
            }
        };

        tracing::debug!(%start_i, "found the end of the buffer");

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

        tracing::debug!(?payload, "playload deserialized");

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
                    Ok(unsafe { std::mem::transmute_copy(&DevNull::INST) })
                } else {
                    Err(Error::IpcError(format!(
                        "Call was successful, but there was no data field: {:?}",
                        std::str::from_utf8(&buf[start_i..])
                    )))
                }
            }

            Payload { error, .. } => Err(Error::IpcError(format!(
                "{} :: {:?} => {}",
                error,
                cmd,
                std::any::type_name::<O>()
            ))),
        }
    }

    async fn writeln(&mut self, b: &[u8]) -> io::Result<usize> {
        let io_slices = [IoSlice::new(b), IoSlice::new(b"\n")];
        self.socket.write_vectored(&io_slices).await
    }

    pub async fn fire<S: AsRef<[u8]>>(&mut self, msg: S) -> io::Result<()> {
        self.writeln(msg.as_ref()).await?;
        Ok(())
    }

    pub async fn compute<C, const N: usize>(&mut self, cmd: C) -> Result<C::Output, Error>
    where
        C: Compute<N>,
        C::Output: DeserializeOwned + Debug + 'static,
    {
        self.mpv_do(cmd.cmd().as_slice()).await
    }

    pub async fn compute_raw<C, D>(&mut self, cmd: D) -> Result<C, Error>
    where
        D: Serialize + Debug,
        C: DeserializeOwned + Debug + 'static,
    {
        self.mpv_do(cmd).await
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
                        line, e
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

    pub fn last(&self) -> LastReference<'_> {
        LastReference { socket: self }
    }
}

pub struct LastReference<'s> {
    socket: &'s LocalMpvSocket<UnixStream>,
}

impl LastReference<'_> {
    fn path(&self) -> PathBuf {
        let mut path = self.socket.path().to_owned();
        let mut name = path
            .file_name()
            .expect("playlist path to have a filename")
            .to_os_string();
        path.pop();
        name.push("_last_queue");
        path.push(name);
        path
    }

    pub async fn fetch(&self) -> Result<Option<usize>, Error> {
        const THREE_HOURS: Duration = Duration::from_secs(60 * 60 * 3);

        let path = self.socket.path();
        let now = SystemTime::now();
        tracing::debug!(?path, "getting m_time on last queue file");
        let modified = match tokio::fs::metadata(&path).await.and_then(|r| r.modified()) {
            Ok(m_time) => m_time,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        tracing::debug!(?modified, ?path, "got m_time on last queue file");
        if (modified.duration_since(now).unwrap_or_default()) > THREE_HOURS
            || modified < self.socket.created_at().await?
        {
            self.reset().await?;
            Ok(None)
        } else {
            match tokio::fs::read_to_string(&path).await {
                Ok(s) => match s.trim().parse() {
                    Ok(n) => Ok(Some(n)),
                    Err(_) => {
                        tracing::error!("failed to parse last queue, file corrupted? '{:?}'", path);
                        Ok(None)
                    }
                },
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e.into()),
            }
        }
    }

    pub async fn reset(&self) -> Result<(), Error> {
        let path = self.path();
        if let Err(e) = tokio::fs::remove_file(&path).await {
            if e.kind() != io::ErrorKind::NotFound {
                return Err(e.into());
            }
        }
        Ok(())
    }

    pub async fn set(&self, u: usize) -> Result<(), Error> {
        let path = self.path();
        tokio::fs::write(path, u.to_string().as_bytes()).await?;
        Ok(())
    }
}

#[derive(Debug)]
struct DevNull {
    _m: (),
}

impl DevNull {
    const INST: Self = DevNull { _m: () };
}

const _: () = assert!(std::mem::size_of::<DevNull>() == 0);

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

impl<T> Display for LocalMpvSocket<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn socket_glob_ends_in_asterisk() {
        assert!(super::SOCKET_GLOB.ends_with('*'))
    }
}
