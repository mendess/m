#![warn(clippy::dbg_macro)]

use serde::{Deserialize, Serialize};
use std::{
    fmt::Display,
    io,
    ops::{Deref, Range},
    path::Path,
};

pub mod downloaded;
#[cfg(feature = "playlist")]
pub mod playlist;
#[cfg(feature = "queue")]
pub mod queue;
#[cfg(feature = "socket")]
pub mod socket;
#[cfg(feature = "ytdl")]
pub mod ytdl;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Link(String);

impl Link {
    const VID_QUERY_PARAM: &'static str = "v=";

    pub fn from_id(s: &LinkId) -> Self {
        Self(format!("https://youtu.be/{}", &s.0))
    }

    pub fn from_url(s: String) -> Result<Self, String> {
        if s.starts_with("http") {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }

    pub fn id(&self) -> &LinkId {
        match self.0.match_indices(Self::VID_QUERY_PARAM).next() {
            Some((i, _)) => {
                let x = &self.0[(i + Self::VID_QUERY_PARAM.len())..];
                let end = x
                    .char_indices()
                    .find_map(|(i, c)| (c == '&').then(|| i))
                    .unwrap_or(x.len());
                LinkId::new(&x[..end])
            }
            None => LinkId::new(
                self.0
                    .split('/')
                    .last()
                    .expect("there should be an id here bro"),
            ),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct LinkId(str);

impl LinkId {
    fn new(s: &str) -> &Self {
        unsafe { std::mem::transmute(s) }
    }
}

impl Deref for LinkId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

pub(crate) fn id_range(s: &str) -> Option<Range<usize>> {
    let front_striped = s.trim_end_matches("=m");
    let start_idx = front_striped.char_indices().rfind(|(_, c)| *c == '=')?.0;
    if front_striped.len() == start_idx {
        return None;
    }
    Some((start_idx + 1)..(front_striped.len()))
}

pub(crate) fn id_from_path<P: AsRef<Path>>(p: &P) -> Option<&LinkId> {
    // format: [name]=[id]=m.ext
    let name = p.as_ref().file_stem()?.to_str()?;
    let range = id_range(name)?;
    Some(LinkId::new(&name[range]))
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
    #[error("failed to read playlist file: {0}")]
    PlaylistFile(String),
}

#[derive(Debug, Clone)]
pub struct Search(String);

impl Search {
    pub fn new(mut s: String) -> Self {
        s.insert_str(0, "ytdl://ytsearch:");
        Self(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod test {
    use crate::id_from_path;
    use std::path::PathBuf;

    #[test]
    fn trivial() {
        assert_eq!(
            Some("AAA"),
            id_from_path(&PathBuf::from("Song Name 😎=AAA=m.mkv"))
        )
    }

    #[test]
    fn no_id() {
        assert_eq!(
            None,
            id_from_path(&PathBuf::from("Some-song-title-AAA.mkv"))
        )
    }

    #[test]
    fn no_id_2() {
        assert_eq!(
            None,
            id_from_path(&PathBuf::from("Some-song-title-AAA=m.mkv"))
        )
    }
}
