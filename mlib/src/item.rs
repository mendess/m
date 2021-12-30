use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fmt::Display,
    ops::{Deref, Range},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    str::FromStr,
};

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
            match Link::from_url(s) {
                Ok(l) => Item::Link(l),
                Err(s) => Item::File(PathBuf::from(s)),
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
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
        id_from_link(&self.0)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }
}

fn id_from_link(s: &str) -> &LinkId {
    match s.match_indices(Link::VID_QUERY_PARAM).next() {
        Some((i, _)) => {
            let x = &s[(i + Link::VID_QUERY_PARAM.len())..];
            let end = x
                .char_indices()
                .find_map(|(i, c)| (c == '&').then(|| i))
                .unwrap_or(x.len());
            LinkId::new(&x[..end])
        }
        None => LinkId::new(s.split('/').last().expect("there should be an id here bro")),
    }
}

impl FromStr for Link {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.starts_with("http") {
            Ok(Self(s.into()))
        } else {
            Err("invalid url")
        }
    }
}

impl Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct LinkId(str);

impl LinkId {
    pub(crate) fn new(s: &str) -> &Self {
        unsafe { std::mem::transmute(s) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_url(s: &str) -> Option<&Self> {
        s.starts_with("http").then(|| id_from_link(s))
    }
}

impl Deref for LinkId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
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

#[derive(Debug, Clone)]
#[repr(transparent)]
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
}

pub fn clean_up_path<P: AsRef<Path>>(p: &P) -> Option<&str> {
    let path = p.as_ref().file_stem()?.to_str()?;
    let range = id_range(path)?;
    Some(&path[..(range.start - 1)])
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
