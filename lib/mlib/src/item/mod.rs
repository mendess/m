pub mod link;
#[cfg(all(feature = "ytdl", feature = "playlist"))]
mod title_cache;

use std::{
    ffi::OsStr,
    fmt::Display,
    ops::Range,
    os::unix::{ffi::OsStrExt, prelude::OsStringExt},
    path::{Path, PathBuf},
    str::Utf8Error,
    string::FromUtf8Error,
};

use derive_more::derive::From;
pub use link::{ChannelLink, Link, PlaylistId, PlaylistLink, VideoId, VideoLink};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::item::link::{BangerId, HasId as _, YtId as _};

#[derive(Debug, Clone, PartialEq, Eq, Hash, From)]
#[from(forward)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Item {
    Link(Link),
    File(PathBuf),
    Search(Search),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemId<'s> {
    VideoId(&'s VideoId),
    BangerId(&'s BangerId),
}

impl<'s> ItemId<'s> {
    pub fn as_str(&self) -> &str {
        match self {
            Self::VideoId(v) => v.as_str(),
            Self::BangerId(b) => b.as_str(),
        }
    }
}

impl Item {
    pub fn id(&self) -> Option<ItemId<'_>> {
        match self {
            Item::Link(l) => l
                .banger_id()
                .map(ItemId::BangerId)
                .or_else(|| l.video_id().map(ItemId::VideoId)),
            Item::File(p) => id_from_path(p),
            Item::Search(_) => None,
        }
    }

    pub fn banger_id(&self) -> Option<&BangerId> {
        match self {
            Item::Link(Link::Banger(l)) => Some(l.id()),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Item::Link(l) => l.as_str().as_bytes(),
            Item::File(f) => f.as_os_str().as_bytes(),
            Item::Search(s) => s.as_str().as_bytes(),
        }
    }

    #[cfg(all(feature = "ytdl", feature = "playlist"))]
    pub async fn fetch_item_title(&self) -> String {
        use crate::ytdl::YtdlBuilder;
        match self {
            Item::Link(l) => match l.as_video() {
                Some(l) => l.resolve_link().await,
                None => match l.as_banger() {
                    Some(l) => l
                        .metadata()
                        .await
                        .map(|t| t.title)
                        .unwrap_or_else(|_| l.to_string()),
                    None => l.to_string(),
                },
            },
            Item::File(f) => clean_up_path(&f)
                .map(ToString::to_string)
                .unwrap_or_else(|| f.to_string_lossy().into_owned()),
            Item::Search(s) => {
                tracing::debug!("fetching title of search {s:?}");
                match title_cache::get_by_search(s).await {
                    Ok(Some(title)) => return title,
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!(?s, error = ?e, "failed to fetch cache of search");
                    }
                };
                let title = YtdlBuilder::new(s)
                    .get_title()
                    .search()
                    .await
                    .map(|b| b.title());

                match title {
                    Ok(title) => {
                        if let Err(e) = title_cache::put_by_search(s, &title).await {
                            tracing::warn!(error = ?e, "failed to cache title");
                        }
                        title
                    }
                    Err(e) => e.to_string(),
                }
            }
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
                    Some(p) => write!(f, "{p}"),
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
                Err(s) => match id_from_path(&s) {
                    Some(ItemId::VideoId(id)) => Item::Link(VideoLink::from_id(id).into()),
                    Some(ItemId::BangerId(id)) => Item::Link(link::BangerLink::from_id(id).into()),
                    None => Self::File(PathBuf::from(s)),
                },
            }
        }
    }
}

pub(crate) fn id_range(s: &str) -> Option<Range<usize>> {
    let front_striped = s.strip_suffix("=m").or_else(|| s.strip_suffix("=mart"))?;
    let start_idx = front_striped.char_indices().rfind(|(_, c)| *c == '=')?.0;
    if front_striped.len() == start_idx {
        return None;
    }
    Some((start_idx + 1)..(front_striped.len()))
}

pub(crate) fn id_from_path<P: AsRef<Path>>(p: &P) -> Option<ItemId<'_>> {
    // format: [name]=[id]=m.ext
    let mut name = p.as_ref();
    let name = loop {
        let new = name.file_stem()?;
        if new == name {
            break name.to_str()?;
        }
        if new.as_bytes().ends_with(b"=m") || new.as_bytes().ends_with(b"=mart") {
            break new.to_str()?;
        }
        name = Path::new(new);
    };
    let range = id_range(name)?;
    if range.len() == 8 {
        Some(ItemId::BangerId(BangerId::new(&name[range])))
    } else {
        Some(ItemId::VideoId(VideoId::new(&name[range])))
    }
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
            Some(ItemId::BangerId(BangerId::new("AAAAAAAA"))),
            id_from_path(&PathBuf::from("Song Name 😎=AAAAAAAA=m.mkv"))
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

    #[test]
    fn art_id() {
        assert_eq!(
            Some(ItemId::BangerId(BangerId::new("AAAAAAAA"))),
            id_from_path(&Path::new("Song Name 😎=AAAAAAAA=mart.jpg"))
        )
    }
}
