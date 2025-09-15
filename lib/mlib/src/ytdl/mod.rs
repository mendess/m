pub mod builder;

use regex::Regex;
use std::sync::LazyLock;
use std::{path::Path, process::ExitStatus};
use thiserror::Error;

pub use builder::{Ytdl, YtdlBuilder, YtdlStream};
use parking_lot::Mutex;
use std::io;
use std::path::PathBuf;
use tokio::process::Command;

#[derive(Error, Debug)]
pub enum YtdlError {
    #[error("not enough fields, expected {expected} found {found}: {fields:?}")]
    InsufisientFields {
        expected: usize,
        found: usize,
        fields: Vec<String>,
    },
    #[error("status {status_code}, because: {stderr}")]
    NonZeroStatus {
        status_code: ExitStatus,
        stderr: String,
    },
}

static ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(v=|youtu.be/)(?P<id>[A-Za-z0-9_\-]{11})").unwrap());

pub fn extract_id(s: &str) -> Option<&str> {
    Some(ID.captures(s)?.name("id").unwrap().as_str())
}

static COOKIES: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn set_cookies_path(path: &Path) {
    *COOKIES.lock() = Some(path.to_path_buf());
}

pub fn custom() -> Command {
    let mut c = Command::new("yt-dlp");
    if let Some(path) = &*COOKIES.lock() {
        c.args([std::ffi::OsStr::new("--cookies"), path.as_os_str()]);
    }
    c
}

pub async fn set_cookies_browser(browser: &str) -> io::Result<()> {
    const DEFAULT_COOKIES_PATH: &str = "/tmp/cookies.txt";

    Command::new("yt-dlp")
        .args([
            "--cookies-from-browser",
            browser,
            "--cookies",
            DEFAULT_COOKIES_PATH,
        ])
        .spawn()?
        .wait()
        .await?;

    if !tokio::fs::try_exists(DEFAULT_COOKIES_PATH).await? {
        return Err(io::Error::other("cookies file wasn't created"));
    }

    set_cookies_path(Path::new(DEFAULT_COOKIES_PATH));

    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn long_url() {
        let url = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
        assert_eq!("jNQXAC9IVRw", extract_id(url).unwrap());
    }

    #[test]
    fn short_url() {
        let url = "https://youtu.be/jNQXAC9IVRw";
        assert_eq!("jNQXAC9IVRw", extract_id(url).unwrap());
    }

    #[test]
    fn dash() {
        let url = "https://www.youtube.com/watch?v=_KhZ7F-jOlI";
        assert_eq!("_KhZ7F-jOlI", extract_id(url).unwrap());
    }
}
