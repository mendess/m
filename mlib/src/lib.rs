#![warn(clippy::dbg_macro)]
#![warn(rust_2018_idioms)]

use std::io;
use thiserror::Error;
use tokio::process::Command;

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
pub(crate) use item::id_from_path;
#[cfg(feature = "items")]
pub use item::{Item, Link, LinkId, Search};

#[derive(Error, Debug)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] io::Error),
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
    #[error("{0}")]
    YtdlError(#[from] ytdl::YtdlError),
    #[error("invalid utf8 {0}")]
    Utf8Error(#[from] std::string::FromUtf8Error),
}

pub async fn update_bar() -> io::Result<()> {
    let mut update_panel = dirs::config_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "config dir not found"))?;
    update_panel.push("m");
    update_panel.push("update_panel.sh");
    tracing::debug!(
        "checking if update panel script (at {}) exists",
        update_panel.display()
    );
    let metadata = tokio::fs::metadata(&update_panel).await;
    tracing::debug!("metadata check for script {:?}", metadata);
    if metadata.is_ok() {
        Command::new("sh").arg(update_panel).spawn()?.wait().await?;
    }

    Ok(())
}
