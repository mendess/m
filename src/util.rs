pub mod notify;
pub mod selector;
pub mod session_kind;

use mlib::item::link::VideoLink;
use mlib::VideoId;
use once_cell::sync::Lazy;
use std::io;
use std::path::PathBuf;
use tokio::process::Command;

pub fn dl_dir() -> anyhow::Result<PathBuf> {
    static PATH: Lazy<Option<PathBuf>> = Lazy::new(|| {
        let mut p = dirs::audio_dir()?;
        p.push("m");
        Some(p)
    });
    PATH.clone()
        .ok_or_else(|| anyhow::anyhow!("couldn't find audio dir"))
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

pub async fn preview_video(l: &VideoId) -> anyhow::Result<()> {
    Command::new("mpv")
        .args(["--start=20", "--geometry=820x466", "--no-terminal"])
        .arg(VideoLink::from_id(l))
        .spawn()?
        .wait()
        .await?;
    Ok(())
}

pub struct RawMode;
impl RawMode {
    pub fn enable() -> crossterm::Result<Self> {
        crossterm::terminal::enable_raw_mode().map(|_| Self)
    }
}
impl Drop for RawMode {
    fn drop(&mut self) {
        if let Err(e) = crossterm::terminal::disable_raw_mode() {
            eprintln!("failed to disable raw mode: {:?}", e);
        } else {
            tracing::trace!("leaving raw mode");
        }
    }
}
