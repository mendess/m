#![warn(clippy::dbg_macro)]
#![warn(rust_2018_idioms)]

use std::{io, path::PathBuf};
use thiserror::Error;

#[cfg(feature = "downloads")]
pub mod downloaded;
#[cfg(feature = "items")]
pub mod item;
#[cfg(feature = "playlist")]
pub mod playlist;
#[cfg(feature = "queue")]
pub mod queue;
#[cfg(feature = "socket")]
pub mod socket;
#[cfg(feature = "ytdl")]
pub mod ytdl;

#[cfg(feature = "items")]
pub use item::{Item, Link, Search, VideoId};

#[derive(Error, Debug)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[cfg(feature = "playlist")]
    #[error("csv: {0}")]
    Csv(#[from] csv_async::Error),

    #[error("no mpv instance running")]
    NoMpvInstance,

    #[error("invalid socket path: {0}")]
    InvalidPath(&'static str),

    #[error("ipc error: {0}")]
    IpcError(String),

    #[error("can't find music directory")]
    MusicDirNotFound,

    #[error("failed to read playlist file: {0}")]
    PlaylistFile(String),

    #[cfg(feature = "ytdl")]
    #[error("{0}")]
    YtdlError(#[from] ytdl::YtdlError),

    #[error("invalid utf8 {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),

    #[cfg(feature = "playlist")]
    #[error("playlist file not found at: {0}")]
    PlaylistFileNotFound(PathBuf),

    #[cfg(feature = "socket")]
    #[error("unexpected error: {0}")]
    UnexpectedError(String),

    #[cfg(feature = "socket")]
    #[error("deserialization error: {0}")]
    Deserialization(String),
}

#[cfg(feature = "socket")]
impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Self::Deserialization(e.to_string())
    }
}

#[cfg(feature = "socket")]
impl From<spark_protocol::RecvError> for Error {
    fn from(e: spark_protocol::RecvError) -> Self {
        use spark_protocol::RecvError;
        match e {
            RecvError::Io(e) => Error::Io(e),
            RecvError::Serde(e) => Error::Deserialization(e.to_string())
        }
    }
}
