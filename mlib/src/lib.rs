#![warn(clippy::dbg_macro)]

use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fmt::Display,
    io,
    ops::{Deref, Range},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
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

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct LinkId(str);

impl LinkId {
    fn new(s: &str) -> &Self {
        unsafe { std::mem::transmute(s) }
    }

    pub fn new_unchecked(s: &str) -> &Self {
        Self::new(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for LinkId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
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
    const PREFIX: &'static str = "ytdl://ytsearch:";
    pub fn new(mut s: String) -> Self {
        s.insert_str(0, Self::PREFIX);
        Self(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub enum Item {
    Link(Link),
    File(PathBuf),
    Search(Search),
}

impl Item {
    pub fn id(&self) -> Option<&LinkId> {
        match self {
            Item::Link(l) => Some(l.id()),
            Item::File(p) => id_from_path(p),
            Item::Search(_) => None,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Item::Link(l) => l.as_str().as_bytes(),
            Item::File(f) => f.as_os_str().as_bytes(),
            Item::Search(s) => s.as_str().as_bytes(),
        }
    }
}

impl AsRef<OsStr> for Item {
    fn as_ref(&self) -> &OsStr {
        match self {
            Item::Link(l) => l.as_str().as_ref(),
            Item::File(p) => p.as_ref(),
            Item::Search(s) => s.as_str().as_ref(),
        }
    }
}

impl Display for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Item::Link(l) => write!(f, "{}", l.as_str()),
            Item::File(p) => {
                let file = p
                    .file_stem()
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_default();
                write!(f, "{}", {
                    match id_range(&file) {
                        Some(range) => &file[..range.start],
                        None => &file,
                    }
                })
            }
            Item::Search(s) => f.write_str(s.as_str()),
        }
    }
}

impl From<String> for Item {
    fn from(s: String) -> Self {
        if s.starts_with(Search::PREFIX) {
            Item::Search(Search(s))
        } else {
            match Link::from_url(s) {
                Ok(l) => Item::Link(l),
                Err(s) => Item::File(PathBuf::from(s)),
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn trivial() {
        assert_eq!(
            Some(LinkId::new("AAA")),
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
