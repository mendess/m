pub mod link;

use std::{
    ffi::OsStr,
    fmt::Display,
    ops::Range,
    os::unix::{ffi::OsStrExt, prelude::OsStringExt},
    path::{Path, PathBuf},
    str::Utf8Error,
    string::FromUtf8Error,
};

pub use link::{Link, PlaylistId, PlaylistLink, VideoId};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use self::link::Id;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Item {
    Link(Link),
    File(PathBuf),
    Search(Search),
}

impl Item {
    pub fn id(&self) -> Option<&VideoId> {
        match self {
            Item::Link(l) => l.video_id(),
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

impl<'s> TryFrom<&'s Item> for &'s str {
    type Error = Utf8Error;

    fn try_from(value: &'s Item) -> Result<Self, Self::Error> {
        match value {
            Item::Link(l) => Ok(l.as_str()),
            Item::File(f) => std::str::from_utf8(f.as_os_str().as_bytes()),
            Item::Search(s) => Ok(s.as_str()),
        }
    }
}

impl TryFrom<Item> for String {
    type Error = FromUtf8Error;

    fn try_from(value: Item) -> Result<Self, Self::Error> {
        match value {
            Item::Link(l) => Ok(l.into_string()),
            Item::File(f) => String::from_utf8(f.into_os_string().into_vec()),
            Item::Search(s) => Ok(s.into_string()),
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
                match clean_up_path(p).or_else(|| p.file_stem().and_then(OsStr::to_str)) {
                    Some(p) => write!(f, "{}", p),
                    None => write!(f, "{}", p.display()),
                }
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
            match Link::try_from(s) {
                Ok(l) => Item::Link(l),
                Err(s) => Item::File(PathBuf::from(s)),
            }
        }
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

pub(crate) fn id_from_path<P: AsRef<Path>>(p: &P) -> Option<&VideoId> {
    // format: [name]=[id]=m.ext
    let name = p.as_ref().file_stem()?.to_str()?;
    let range = id_range(name)?;
    Some(VideoId::new(&name[range]))
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[repr(transparent)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Search(String);

impl Search {
    const PREFIX: &'static str = "ytdl://ytsearch";
    pub fn new(mut s: String) -> Self {
        s.insert(0, ':');
        s.insert_str(0, Self::PREFIX);
        Self(s)
    }

    pub fn multiple(s: String, limit: usize) -> Self {
        Self(format!("{}{}:{}", Self::PREFIX, limit, s))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn into_string(self) -> String {
        self.0
    }
}

pub fn clean_up_path<P: AsRef<Path>>(p: &P) -> Option<&str> {
    if p.as_ref().starts_with("http") {
        None
    } else {
        let path = p.as_ref().file_stem()?.to_str()?;
        let range = id_range(path)?;
        Some(&path[..(range.start - 1)])
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn trivial() {
        assert_eq!(
            Some(VideoId::new("AAA")),
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
