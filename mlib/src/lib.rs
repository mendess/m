#![warn(clippy::dbg_macro)]

use std::{io, path::Path};

pub mod downloaded;
#[cfg(feature = "playlist")]
pub mod playlist;
#[cfg(feature = "queue")]
pub mod queue;
#[cfg(feature = "socket")]
pub mod socket;
#[cfg(feature = "ytdl")]
pub mod ytdl;

#[derive(Debug)]
pub struct Link(String);

impl Link {
    const VID_QUERY_PARAM: &'static str = "v=";

    pub fn from_id(s: &str) -> Self {
        Self(format!("https://youtu.be/{}", s))
    }

    pub fn from_url(s: String) -> Self {
        Self(s)
    }

    pub fn id(&self) -> &str {
        match self.0.match_indices(Self::VID_QUERY_PARAM).next() {
            Some((i, _)) => {
                let x = &self.0[(i + Self::VID_QUERY_PARAM.len())..];
                let end = x
                    .char_indices()
                    .find_map(|(i, c)| (c == '&').then(|| i))
                    .unwrap_or(x.len());
                &x[..end]
            }
            None => self
                .0
                .split('/')
                .last()
                .expect("there should be an id here bro"),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

pub(crate) fn id_from_path<P: AsRef<Path>>(p: &P) -> Option<&str> {
    // format: [name]=[id]=m.ext
    let name = p.as_ref().file_stem()?.to_str()?;
    let front_striped = name.trim_end_matches("=m");
    let start_idx = front_striped.char_indices().rfind(|(_, c)| *c == '=')?.0;
    if front_striped.len() == start_idx {
        return None;
    }
    Some(&front_striped[(start_idx + 1)..])
}

#[derive(thiserror::Error, Debug)]
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
}
