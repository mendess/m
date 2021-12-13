use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use regex::Regex;
use thiserror::Error;
use tokio::net::UnixStream;

static SOCKET_GLOB: Lazy<String> = Lazy::new(|| format!("/tmp/{}/.mpvsocket*", whoami::username()));

static SOCKET_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\.mpvsocket([0-9]+)$").unwrap());

#[derive(Error, Debug)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),
    #[error("invalid socket path: {0}")]
    InvalidPath(&'static str),
}

pub async fn most_recent_cached() -> Result<UnixStream, Error> {
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
        let (path, sock) = most_recent().await?;
        tracing::trace!("Cache hit {}", path.display());
        current.0 = path;
        current.1 = Instant::now();
        Ok(sock)
    } else {
        current.1 = Instant::now();
        match UnixStream::connect(&current.0).await {
            Ok(sock) => Ok(sock),
            Err(_) => {
                let (path, sock) = most_recent().await?;
                tracing::trace!("Cache miss. Opening {} instead", path.display());
                current.0 = path;
                Ok(sock)
            }
        }
    }
}

pub async fn new() -> Result<PathBuf, Error> {
    fn new_path(end: &str) -> PathBuf {
        let mut new_path = SOCKET_GLOB.clone();
        new_path.pop(); // remove '*'
        new_path.push_str(end);
        PathBuf::from(new_path)
    }

    let path = match most_recent().await {
        Ok((p, _)) => p,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(new_path("0"));
        }
        Err(e) => return Err(e.into()),
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

pub async fn most_recent() -> io::Result<(PathBuf, UnixStream)> {
    let mut available_sockets: Vec<_> = glob::glob(&*SOCKET_GLOB)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|x| x.to_str().map(|x| SOCKET_REGEX.is_match(x)) == Some(true))
        .collect();
    available_sockets.sort();
    for s in available_sockets.into_iter().rev() {
        if let Ok(sock) = UnixStream::connect(&s).await {
            return Ok((s, sock));
        }
    }
    Err(io::ErrorKind::NotFound.into())
}
