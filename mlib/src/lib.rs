#![warn(clippy::dbg_macro)]
#![warn(rust_2018_idioms)]

#[cfg(feature = "downloads")]
pub mod downloaded;
pub mod item;
#[cfg(feature = "player-connection")]
pub mod players;
#[cfg(feature = "playlist")]
pub mod playlist;
#[cfg(feature = "queue")]
pub mod queue;
#[cfg(feature = "statistics")]
pub mod statistics;
#[cfg(feature = "ytdl")]
pub mod ytdl;

pub use item::{Item, Link, Search, VideoId};

#[cfg(any(feature = "ytdl", feature = "playlist", feature = "player-connection"))]
#[derive(Debug)]
#[cfg_attr(
    any(feature = "ytdl", feature = "playlist", feature = "player-connection"),
    derive(thiserror::Error)
)]
pub enum Error {
    #[cfg(any(feature = "ytdl", feature = "player-connection", feature = "playlist"))]
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[cfg(feature = "playlist")]
    #[error("csv: {0}")]
    Csv(#[from] csv_async::Error),

    #[cfg(feature = "player-connection")]
    #[error("libmpv error: {0}")]
    MpvError(players::error::MpvError),

    #[cfg(feature = "playlist")]
    #[error("failed to read playlist file: {0}")]
    PlaylistFile(String),

    #[cfg(feature = "ytdl")]
    #[error("{0}")]
    YtdlError(#[from] ytdl::YtdlError),

    #[cfg(feature = "playlist")]
    #[error("playlist file not found at: {0}")]
    PlaylistFileNotFound(std::path::PathBuf),
}

#[cfg(feature = "player-connection")]
impl From<players::error::Error> for Error {
    fn from(e: players::error::Error) -> Self {
        match e {
            players::error::Error::Io(e) => Self::Io(e),
            players::error::Error::Mpv(e) => Self::MpvError(e),
        }
    }
}
