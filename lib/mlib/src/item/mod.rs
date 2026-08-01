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
    time::Duration,
};

use derive_more::derive::From;
pub use link::{
    ChannelLink, Link, PlaylistId, PlaylistLink, VideoId, VideoLink,
};
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

    pub fn to_link(&self) -> Link {
        match self {
            ItemId::VideoId(id) => Link::from(id.to_link()),
            ItemId::BangerId(id) => Link::from(id.to_link()),
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

    pub fn as_str(&self) -> &str {
        match self {
            Item::Link(l) => l.as_str(),
            Item::File(f) => f.as_os_str().to_str().unwrap(),
            Item::Search(s) => s.as_str(),
        }
    }

    #[cfg(all(feature = "ytdl", feature = "playlist"))]
    pub async fn runtime(&self) -> Result<Duration, crate::Error> {
        use crate::ytdl::YtdlBuilder;

        match self {
            Item::Link(link) => link.runtime().await,
            Item::File(path_buf) => {
                if tokio::fs::metadata(path_buf).await?.is_dir() {
                    // TODO
                    tracing::warn!(?path_buf, "can't get runtime of directory");
                    return Ok(Duration::ZERO);
                }
                // ffprobe -i <file> -show_entries format=duration -v quiet -of csv=p=0

                use tokio::process::Command;
                let duration_output = Command::new("ffprobe")
                    .arg("-i")
                    .arg(path_buf)
                    .args([
                        "-show_entries",
                        "format=duration",
                        "-v",
                        "quiet",
                        "-of",
                        "csv=p=0",
                    ])
                    .output()
                    .await?;
                if !duration_output.status.success() {
                    return Err(crate::Error::Io(std::io::Error::other(
                        format!(
                            "ffprobe for {} failed with status: {:?}",
                            path_buf.display(),
                            duration_output.status.code(),
                        ),
                    )));
                }
                let duration_str = std::str::from_utf8(&duration_output.stdout)
                    .map_err(|_| {
                        std::io::Error::other(format!(
                            "ffprobe for {} output is not utf8",
                            path_buf.display()
                        ))
                    })?;

                let duration = duration_str.trim().parse::<f32>().map_err(|_| {
                    std::io::Error::other(format!(
                        "ffprobe for {} output is not a float: {duration_str:?}",
                        path_buf.display()
                    ))
                })?;

                Ok(Duration::from_secs_f32(duration))
            }
            Item::Search(search) => Ok(YtdlBuilder::new(search)
                .get_duration()
                .search()
                .await?
                .duration()),
        }
    }

    #[cfg(all(feature = "ytdl", feature = "playlist"))]
    pub async fn fetch_item_title<'s>(
        &'s self,
        playlist: &'s crate::playlist::Playlist,
    ) -> std::borrow::Cow<'s, str> {
        use crate::ytdl::YtdlBuilder;
        use std::borrow::Cow;
        match self {
            Item::Link(l) => match l.as_video() {
                Some(l) => l.resolve_link().await.into(),
                None => match l.as_banger() {
                    Some(l) => match playlist.find_by_id(l.id()) {
                        Some(s) => Cow::Borrowed(&s.name),
                        None => l
                            .metadata()
                            .await
                            .map(|t| t.title.into())
                            .unwrap_or_else(|_| Cow::Borrowed(l.as_str())),
                    },
                    None => Cow::Borrowed(l.as_str()),
                },
            },
            Item::File(f) => clean_up_path(f)
                .map(Cow::Borrowed)
                .or_else(|| f.file_stem()?.to_str().map(Cow::Borrowed))
                .unwrap_or_else(|| f.to_string_lossy().into_owned().into()),
            Item::Search(s) => {
                tracing::debug!("fetching title of search {s:?}");
                match title_cache::get_by_search(s).await {
                    Ok(Some(title)) => return title.into(),
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
                        if let Err(e) =
                            title_cache::put_by_search(s, &title).await
                        {
                            tracing::warn!(error = ?e, "failed to cache title");
                        }
                        title.into()
                    }
                    Err(e) => e.to_string().into(),
                }
            }
        }
    }

    #[cfg(all(feature = "ytdl", feature = "playlist"))]
    pub async fn fetch_item_artist<'s>(
        &'s self,
        playlist: &'s crate::playlist::Playlist,
    ) -> Option<std::borrow::Cow<'s, str>> {
        use std::borrow::Cow;
        match self {
            Item::Link(l) => match l.as_video() {
                Some(_) => None, // TODO artist cache needed for this
                None => match l.as_banger() {
                    Some(l) => match playlist.find_by_id(l.id()) {
                        Some(s) => s.artist.as_deref().map(Cow::Borrowed),
                        None => None,
                    },
                    None => None,
                },
            },
            Item::File(f) if f.metadata().is_ok_and(|f| f.is_file()) => {
                get_artist_from_tags(f).await.map(Cow::Owned)
            }
            Item::File(_) => None,
            Item::Search(_) => {
                // TODO artist cache needed for this
                None
            }
        }
    }
}

#[tracing::instrument(level = "warn")]
#[cfg(all(feature = "ytdl", feature = "playlist"))]
async fn get_artist_from_tags(file: &Path) -> Option<String> {
    if let Some("jpg" | "png") = file.extension().and_then(OsStr::to_str) {
        return None;
    }
    let output = tokio::process::Command::new("ffprobe")
        .arg(file)
        .args([
            "-of",
            "json",
            "-hide_banner",
            "-show_entries",
            "stream_tags:format_tags",
        ])
        .output()
        .await;
    let output = match output {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(error = ?e, "failed to ffprobe");
            return None;
        }
    };
    #[derive(Deserialize)]
    struct Metadata {
        format: Format,
    }
    #[derive(Deserialize)]
    struct Format {
        tags: Tags,
    }
    #[derive(Deserialize)]
    struct Tags {
        #[serde(alias = "ARTIST")]
        #[serde(alias = "Artist")]
        artist: String,
    }
    let Metadata {
        format: Format {
            tags: Tags { artist },
        },
    } = match serde_json::from_slice::<Metadata>(&output.stdout) {
        Ok(meta) => meta,
        Err(e) => {
            tracing::warn!(error = ?e, "failed to deserialize metadata");
            return None;
        }
    };
    Some(artist)
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

pub fn path_to_title(path: &Path) -> String {
    match clean_up_path(&path)
        .or_else(|| path.file_stem().and_then(OsStr::to_str))
    {
        Some(p) => p.to_string(),
        None => format!("{}", path.display()),
    }
}

impl Display for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Item::Link(l) => write!(f, "{}", l.as_str()),
            Item::File(p) => write!(f, "{}", p.display()),
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
                    Some(ItemId::VideoId(id)) => {
                        Item::Link(VideoLink::from_id(id).into())
                    }
                    Some(ItemId::BangerId(id)) => {
                        Item::Link(link::BangerLink::from_id(id).into())
                    }
                    None => Self::File(PathBuf::from(s)),
                },
            }
        }
    }
}

pub(crate) fn id_range(s: &str) -> Option<Range<usize>> {
    let front_striped =
        s.strip_suffix("=m").or_else(|| s.strip_suffix("=mart"))?;
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
        if new.as_bytes().ends_with(b"=m") || new.as_bytes().ends_with(b"=mart")
        {
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

    pub fn into_string(self) -> String {
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
