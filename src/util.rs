pub mod notify;
pub mod selector;
pub mod session_kind;
pub mod with_video;

use mlib::item::link::VideoLink;
use mlib::VideoId;
use once_cell::sync::Lazy;
use std::fmt::Display;
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;

#[derive(Debug)]
pub enum DisplayEither<A, B> {
    Left(A),
    Right(B),
}

impl<A, B> Display for DisplayEither<A, B>
where
    A: Display,
    B: Display,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DisplayEither::Left(l) => write!(f, "{l}"),
            DisplayEither::Right(r) => write!(f, "{r}"),
        }
    }
}

pub struct DurationFmt(pub Duration);

impl Display for DurationFmt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.0.as_secs();
        let (h, s) = (s / 3600, s % 3600);
        let (m, s) = (s / 60, s % 60);
        if h > 0 {
            write!(f, "{:02}:", h)?;
        }
        write!(f, "{:02}:{:02}", m, s)
    }
}

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
