use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::{ffi::OsStr, fmt::Display, ops::Deref, str::FromStr};

pub trait IntoPlaylist {
    fn into_playlist(self) -> PlaylistLink;
}

pub trait IntoVideo {
    fn into_video(self) -> VideoLink;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Link {
    Video(VideoLink),
    Playlist(PlaylistLink),
}

impl Link {
    const QUERY_VID_PARAM: &'static str = "v=";
    const QUERY_PLAYLIST_PARAM: &'static str = "list=";

    pub fn from_video_id(id: &VideoId) -> Self {
        Self::Video(VideoLink::from_id(id))
    }

    pub fn from_playlist_id(id: &PlaylistId) -> Self {
        Self::Playlist(PlaylistLink::from_id(id))
    }

    pub fn from_url(s: String) -> Result<Self, String> {
        PlaylistLink::from_url(s)
            .map(Self::Playlist)
            .or_else(|s| VideoLink::from_url(s).map(Self::Video))
    }

    pub fn video_id(&self) -> Option<&VideoId> {
        match self {
            Self::Video(l) => Some(l.id()),
            Self::Playlist(l) => l.video_id(),
        }
    }

    pub fn playlist_id(&self) -> Option<&PlaylistId> {
        match self {
            Self::Video(_) => None,
            Self::Playlist(l) => Some(l.id()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Video(l) => l.as_str(),
            Self::Playlist(l) => l.as_str(),
        }
    }

    pub fn into_string(self) -> String {
        match self {
            Self::Video(l) => l.into_string(),
            Self::Playlist(l) => l.into_string(),
        }
    }

    pub fn as_video(&self) -> Result<&VideoLink, &PlaylistLink> {
        match self {
            Self::Video(l) => Ok(l),
            Self::Playlist(l) => l.as_video_link(),
        }
    }

    pub fn into_video(self) -> Result<VideoLink, PlaylistLink> {
        match self {
            Self::Video(l) => Ok(l),
            Self::Playlist(l) => l.into_video_link(),
        }
    }

    pub fn as_playlist(&self) -> Option<&PlaylistLink> {
        match self {
            Self::Video(_) => None,
            Self::Playlist(l) => Some(l),
        }
    }

    pub fn as_playlist_mut(&mut self) -> Option<&mut PlaylistLink> {
        match self {
            Self::Video(_) => None,
            Self::Playlist(l) => Some(l),
        }
    }
}

impl From<VideoLink> for Link {
    fn from(l: VideoLink) -> Self {
        Self::Video(l)
    }
}

impl FromStr for Link {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        s.parse()
            .map(Self::Playlist)
            .or_else(|_| s.parse().map(Self::Video))
    }
}

fn id_from_link<'s>(s: &'s str, param: &'static str) -> Option<&'s str> {
    match s.match_indices(param).next() {
        Some((i, _)) => {
            let x = &s[(i + param.len())..];
            let end = x
                .char_indices()
                .find_map(|(i, c)| (c == '&').then(|| i))
                .unwrap_or(x.len());
            Some(&x[..end])
        }
        None => s.split('/').last(),
    }
}

impl Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Video(v) => v.fmt(f),
            Self::Playlist(v) => v.fmt(f),
        }
    }
}

impl AsRef<OsStr> for Link {
    fn as_ref(&self) -> &OsStr {
        match self {
            Self::Video(l) => l.as_ref(),
            Self::Playlist(l) => l.as_ref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct VideoLink(String);

impl VideoLink {
    const QUERY_PARAM: &'static str = "v=";

    pub fn id(&self) -> &VideoId {
        VideoId::new(
            id_from_link(&self.0, Self::QUERY_PARAM).expect("video link to have a video id"),
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_url(s: String) -> Result<Self, String> {
        if Self::is_video_link(&s) {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }

    pub fn from_id(s: &VideoId) -> Self {
        Self(format!("https://youtu.be/{}", &s.0))
    }

    pub fn into_string(self) -> String {
        self.0
    }

    fn is_video_link(s: &str) -> bool {
        static SHORT_LINK_PAT: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"https?://youtu\.be/[0-9a-zA-Z_\-]+$").unwrap());

        s.starts_with("http") && (s.contains(Self::QUERY_PARAM) || SHORT_LINK_PAT.is_match(s))
    }

    pub fn shorten(&mut self) {
        if self.0.contains("youtube") {
            use std::fmt::Write;
            let id = self.id().as_str().to_owned();
            self.0.clear();
            let _ = write!(self.0, "https://youtu.be/{id}");
        }
    }
}

impl FromStr for VideoLink {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        VideoLink::is_video_link(s)
            .then(|| VideoLink(s.into()))
            .ok_or("invalid video url")
    }
}

impl Display for VideoLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<OsStr> for VideoLink {
    fn as_ref(&self) -> &OsStr {
        self.0.as_ref()
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct VideoId(str);

impl VideoId {
    pub(crate) fn new(s: &str) -> &Self {
        unsafe { std::mem::transmute(s) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_url(s: &str) -> Option<&Self> {
        s.starts_with("http")
            .then(|| id_from_link(s, Link::QUERY_VID_PARAM))
            .flatten()
            .map(Self::new)
    }
}

impl Deref for VideoId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

/*
 * https://youtu.be/UpIBKNxSeZU
 * https://www.youtube.com/playlist?list=PL17PSucW5L7nEPyX3tqEzq_wmyYk2IkXr
 * https://www.youtube.com/watch?v=UpIBKNxSeZU&list=PL17PSucW5L7nEPyX3tqEzq_wmyYk2IkXr
 * https://www.youtube.com/watch?v=UpIBKNxSeZU
 */
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlaylistLink(String);

impl PlaylistLink {
    const QUERY_PARAM: &'static str = "list=";

    fn is_playlist_link(s: &str) -> bool {
        s.starts_with("http") && s.contains(Self::QUERY_PARAM)
    }

    pub fn from_url(s: String) -> Result<Self, String> {
        if PlaylistLink::is_playlist_link(&s) {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }

    pub fn from_id(s: &PlaylistId) -> Self {
        Self(format!(
            "https://youtube.com/playlist?{}{}",
            Self::QUERY_PARAM,
            &s.0
        ))
    }

    pub fn id(&self) -> &PlaylistId {
        PlaylistId::new(
            id_from_link(&self.0, Self::QUERY_PARAM).expect("playlist link to have a playlist id"),
        )
    }

    pub fn video_id(&self) -> Option<&VideoId> {
        id_from_link(&self.0, VideoLink::QUERY_PARAM).map(VideoId::new)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_string(self) -> String {
        self.0
    }

    pub fn without_video_id(&self) -> Self {
        static LINK_REGEX: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"watch\?.*(list=[^&]+)").unwrap());
        let new = if let Some(captures) = LINK_REGEX.captures(&self.0) {
            let start = captures.get(0).unwrap().start();
            let list_id = captures.get(1).unwrap().as_str();
            let mut new = String::with_capacity(start + "playlist?".len() + list_id.len());
            new.push_str(&self.0[..start]);
            new.push_str("playlist?");
            new.push_str(list_id);
            new
        } else {
            self.0.clone()
        };
        Self(new)
    }

    pub fn into_video_link(self) -> Result<VideoLink, Self> {
        VideoLink::from_url(self.0).map_err(Self)
    }

    pub fn as_video_link(&self) -> Result<&VideoLink, &Self> {
        if VideoLink::is_video_link(&self.0) {
            Ok(unsafe { std::mem::transmute(self) })
        } else {
            Err(self)
        }
    }
}

impl FromStr for PlaylistLink {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::is_playlist_link(s)
            .then(|| Self(s.into()))
            .ok_or("invalid playlist link")
    }
}

impl Display for PlaylistLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<OsStr> for PlaylistLink {
    fn as_ref(&self) -> &OsStr {
        self.0.as_ref()
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct PlaylistId(str);

impl PlaylistId {
    pub(crate) fn new(s: &str) -> &Self {
        unsafe { std::mem::transmute(s) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_url(s: &str) -> Option<&Self> {
        s.starts_with("http")
            .then(|| id_from_link(s, Link::QUERY_PLAYLIST_PARAM))
            .flatten()
            .map(Self::new)
    }
}

impl Deref for PlaylistId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn strip_video_id() {
        const BEFORE: &str =
            "https://www.youtube.com/watch?v=UpIBKNxSeZU&list=PL17PSucW5L7nEPyX3tqEzq_wmyYk2IkXr";
        const AFTER: &str =
            "https://www.youtube.com/playlist?list=PL17PSucW5L7nEPyX3tqEzq_wmyYk2IkXr";

        let playlist_link = PlaylistLink::from_url(BEFORE.to_string())
            .unwrap()
            .without_video_id();
        assert_eq!(AFTER, playlist_link.as_str());
    }
}
