#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
use std::{borrow::Cow, ffi::OsStr, fmt::Display, ops::Deref, str::FromStr};
use url::Url;

pub trait IntoPlaylist {
    fn into_playlist(self) -> PlaylistLink;
}

pub trait IntoVideo {
    fn into_video(self) -> VideoLink;
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Link {
    Video(VideoLink),
    Playlist(PlaylistLink),
    OtherPlatform(url::Url),
}

impl Link {
    pub fn from_video_id(id: &VideoId) -> Self {
        Self::Video(VideoLink::from_id(id))
    }

    pub fn from_playlist_id(id: &PlaylistId) -> Self {
        Self::Playlist(PlaylistLink::from_id(id))
    }

    pub fn from_url(s: Url) -> Self {
        PlaylistLink::from_url(s)
            .map(Self::Playlist)
            .or_else(|s| VideoLink::from_url(s).map(Self::Video))
            .unwrap_or_else(Self::OtherPlatform)
    }

    pub fn video_id(&self) -> Option<&VideoId> {
        match self {
            Self::Video(l) => Some(l.id()),
            Self::Playlist(l) => l.video_id(),
            Self::OtherPlatform(_) => None,
        }
    }

    pub fn playlist_id(&self) -> Option<&PlaylistId> {
        match self {
            Self::Video(_) => None,
            Self::Playlist(l) => Some(l.id()),
            Self::OtherPlatform(_) => None,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Video(l) => l.as_str(),
            Self::Playlist(l) => l.as_str(),
            Self::OtherPlatform(url) => url.as_str(),
        }
    }

    pub fn into_string(self) -> String {
        match self {
            Self::Video(l) => l.into_string(),
            Self::Playlist(l) => l.into_string(),
            Self::OtherPlatform(url) => url.into(),
        }
    }

    pub fn as_video(&self) -> Option<&VideoLink> {
        match self {
            Self::Video(l) => Some(l),
            Self::Playlist(l) => l.as_video_link().ok(),
            Self::OtherPlatform(_) => None,
        }
    }

    pub fn into_video(self) -> Result<VideoLink, Self> {
        match self {
            Self::Video(l) => Ok(l),
            Self::Playlist(l) => l.into_video_link().map_err(Self::Playlist),
            Self::OtherPlatform(url) => Err(Self::OtherPlatform(url)),
        }
    }

    pub fn as_playlist(&self) -> Option<&PlaylistLink> {
        match self {
            Self::Video(_) => None,
            Self::Playlist(l) => Some(l),
            Self::OtherPlatform(_) => None,
        }
    }

    pub fn as_playlist_mut(&mut self) -> Option<&mut PlaylistLink> {
        match self {
            Self::Video(_) => None,
            Self::Playlist(l) => Some(l),
            Self::OtherPlatform(_) => None,
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
            .or_else(|_| {
                s.parse()
                    .map(Self::OtherPlatform)
                    .map_err(|_| "invalid url")
            })
    }
}

fn id_from_link<T: Id + ?Sized>(s: &Url) -> Option<&T> {
    s.query_pairs()
        .find(|(k, _)| k == T::QUERY_PARAM)
        .map(|(_, id)| match id {
            Cow::Owned(_) => unreachable!("yt ids are always url safe"),
            Cow::Borrowed(id) => T::new(id),
        })
}

impl Display for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Video(v) => v.fmt(f),
            Self::Playlist(v) => v.fmt(f),
            Self::OtherPlatform(v) => v.fmt(f),
        }
    }
}

impl AsRef<OsStr> for Link {
    fn as_ref(&self) -> &OsStr {
        match self {
            Self::Video(l) => l.as_ref(),
            Self::Playlist(l) => l.as_ref(),
            Self::OtherPlatform(url) => url.as_str().as_ref(),
        }
    }
}

impl TryFrom<String> for Link {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Url::parse(&value).ok().map(Self::from_url).ok_or(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct VideoLink(Url);

impl VideoLink {
    pub fn id(&self) -> &VideoId {
        id_from_link(&self.0)
            .or_else(|| VideoId::from_short_url(&self.0))
            .expect("video link to have a video id")
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn from_url(s: Url) -> Result<Self, Url> {
        if Self::is_video_link(&s) {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }

    pub fn from_id(s: &VideoId) -> Self {
        let mut base = Url::parse("https://youtu.be/").unwrap();
        base.set_path(&s.0);
        Self(base)
    }

    pub fn into_string(self) -> String {
        self.0.into()
    }

    fn is_video_link(s: &Url) -> bool {
        s.scheme().starts_with("http")
            && (s.host_str().is_some_and(|host| host.contains("youtu.be"))
                || s.query_pairs().any(|(k, _)| k == VideoId::QUERY_PARAM))
    }

    pub fn shorten(&mut self) {
        if self
            .0
            .domain()
            .is_some_and(|domain| domain.contains("youtube"))
        {
            let id = self.id().as_str().to_owned();
            self.0.set_host(Some("youtu.be")).unwrap();
            self.0.set_path(&id);
            self.0.set_query(None);
        }
    }
}

impl FromStr for VideoLink {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url = Url::parse(s).map_err(|_| "not a url")?;
        Self::from_url(url).map_err(|_| "not a video url")
    }
}

impl Display for VideoLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl AsRef<OsStr> for VideoLink {
    fn as_ref(&self) -> &OsStr {
        self.0.as_str().as_ref()
    }
}

impl TryFrom<String> for VideoLink {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Url::parse(&value)
            .ok()
            .and_then(|url| Self::from_url(url).ok())
            .ok_or(value)
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct VideoId(str);

impl VideoId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_url(s: &Url) -> Option<&Self> {
        id_from_link(s).or_else(|| Self::from_short_url(s))
    }

    fn from_short_url(url: &Url) -> Option<&Self> {
        url.host_str()
            .is_some_and(|h| h.contains("youtu.be"))
            .then(|| url.path().trim_start_matches('/'))
            .map(VideoId::new)
    }
}

impl Deref for VideoId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl Id for VideoId {
    const QUERY_PARAM: &'static str = "v";
    fn new(s: &str) -> &Self {
        unsafe { std::mem::transmute(s) }
    }
}

/*
 * https://youtu.be/UpIBKNxSeZU
 * https://www.youtube.com/playlist?list=PL17PSucW5L7nEPyX3tqEzq_wmyYk2IkXr
 * https://www.youtube.com/watch?v=UpIBKNxSeZU&list=PL17PSucW5L7nEPyX3tqEzq_wmyYk2IkXr
 * https://www.youtube.com/watch?v=UpIBKNxSeZU
 */
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct PlaylistLink(Url);

impl PlaylistLink {
    // fn is_playlist_link(s: &str) -> bool {
    //     s.starts_with("http") && s.contains(Self::QUERY_PARAM)
    // }

    pub fn from_url(s: Url) -> Result<Self, Url> {
        if s.scheme().starts_with("http")
            && s.query_pairs().any(|(k, _)| k == PlaylistId::QUERY_PARAM)
        {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }

    pub fn from_id(s: &PlaylistId) -> Self {
        let mut base = Url::parse("https://youtube.com/playlist").unwrap();
        base.query_pairs_mut()
            .append_pair(PlaylistId::QUERY_PARAM, &s.0);
        Self(base)
    }

    pub fn id(&self) -> &PlaylistId {
        self.0
            .query_pairs()
            .find(|(k, _)| k == PlaylistId::QUERY_PARAM)
            .map(|(_, id)| match id {
                Cow::Owned(_) => unreachable!("yt ids are always url safe"),
                Cow::Borrowed(id) => PlaylistId::new(id),
            })
            .unwrap()
    }

    pub fn video_id(&self) -> Option<&VideoId> {
        id_from_link(&self.0)
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn into_string(self) -> String {
        self.0.into()
    }

    pub fn without_video_id(&self) -> Self {
        let mut new = self.0.clone();
        new.set_path("playlist");
        new.query_pairs_mut()
            .clear()
            .append_pair(PlaylistId::QUERY_PARAM, &self.id().0);
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
        let url = Url::parse(s).map_err(|_| "not a url")?;
        Self::from_url(url).map_err(|_| "invalid playlist link")
    }
}

impl Display for PlaylistLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl AsRef<OsStr> for PlaylistLink {
    fn as_ref(&self) -> &OsStr {
        self.0.as_str().as_ref()
    }
}

impl TryFrom<String> for PlaylistLink {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Url::parse(&value)
            .ok()
            .and_then(|url| Self::from_url(url).ok())
            .ok_or(value)
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct PlaylistId(str);

impl PlaylistId {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_url(url: &Url) -> Option<&Self> {
        url.scheme()
            .starts_with("http")
            .then(|| id_from_link(url))
            .flatten()
    }
}

impl Id for PlaylistId {
    const QUERY_PARAM: &'static str = "list";
    fn new(s: &str) -> &Self {
        unsafe { std::mem::transmute(s) }
    }
}

impl Deref for PlaylistId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

pub(crate) trait Id {
    const QUERY_PARAM: &'static str;

    fn new(s: &str) -> &Self;
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

        let playlist_link = PlaylistLink::from_url(BEFORE.parse().unwrap())
            .unwrap()
            .without_video_id();
        assert_eq!(AFTER, playlist_link.as_str());
    }
}
