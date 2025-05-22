pub mod notify;
pub mod prompt;
pub mod session_kind;
pub mod with_video;

use mlib::VideoId;
use mlib::item::link::VideoLink;
use std::fmt::Display;
use std::io::{self, StdoutLock, Write as _};
use std::path::PathBuf;
use std::time::Duration;
use tokio::process::Command;
use tokio::sync::OnceCell;

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

pub async fn dl_dir() -> anyhow::Result<PathBuf> {
    static PATH: OnceCell<PathBuf> = OnceCell::const_new();

    PATH.get_or_try_init(|| async {
        let mut p = dirs::audio_dir().ok_or_else(|| anyhow::anyhow!("couldn't find audio dir"))?;
        p.push("m");
        tokio::fs::create_dir_all(&p).await?;
        Ok(p)
    })
    .await
    .cloned()
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
    pub fn enable() -> io::Result<Self> {
        crossterm::terminal::enable_raw_mode().map(|_| Self)
    }

    pub fn guarantee_space(
        &self,
        stdout: &mut StdoutLock<'static>,
        required_space: u16,
    ) -> io::Result<(u16, u16)> {
        use crossterm::QueueableCommand as _;
        use crossterm::terminal::ScrollUp;

        let (_, rows) = crossterm::terminal::size()?;
        let start_position @ (cursor_x, cursor_y) = crossterm::cursor::position()?;
        let missing_rows = required_space.saturating_sub(rows.saturating_sub(cursor_y + 1));
        if missing_rows != 0 {
            stdout.queue(ScrollUp(missing_rows))?.flush()?;
            Ok((cursor_x, cursor_y - missing_rows))
        } else {
            Ok(start_position)
        }
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
