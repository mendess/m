#![warn(clippy::dbg_macro)]
#![warn(rust_2018_idioms)]

use std::io;
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
}
