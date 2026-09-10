use derive_more::derive::From;
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "downloads")]
use std::time::Duration;
use std::{borrow::Cow, ops::Deref};
use url::Url;

pub trait IntoPlaylist {
    fn into_playlist(self) -> PlaylistLink;
}

pub trait IntoVideo {
    fn into_video(self) -> VideoLink;
}

pub trait HasId {
    type Id: AsRef<str> + ?Sized;

    fn id(&self) -> &Self::Id;
}

pub trait YtId {
    const QUERY_PARAM: &'static str;

    fn new(s: &str) -> &Self;

    fn boxed(&self) -> Box<Self>;
}

macro_rules! impl_link {
    ($($type:ty),*$(,)?) => {
        $(
            static_assertions::assert_impl_all!($type: TryFrom<url::Url, Error = url::Url>, AsRef<str>);

            impl TryFrom<String> for $type {
                type Error = String;
                fn try_from(value: String) -> Result<Self, Self::Error> {
                    value.parse().ok().and_then(|url| <Self as TryFrom<Url>>::try_from(url).ok()).ok_or(value)
                }
            }

            impl std::fmt::Display for $type {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    let s: &str = self.as_ref();
                    write!(f, "{s}")
                }
            }

            impl AsRef<std::ffi::OsStr> for $type {
                fn as_ref(&self) -> &std::ffi::OsStr {
                    let s: &str = self.as_ref();
                    s.as_ref()
                }
            }

            impl ::std::str::FromStr for $type {
                type Err = &'static str;

                fn from_str(s: &str) -> Result<Self, Self::Err> {
                    Self::try_from(
                            s
                            .parse::<url::Url>()
                            .map_err(|_| "not a valid url")?
                        )
                        .map_err(|_| ::std::concat!("not a valid ", ::std::stringify!($type)))
                }
            }
        )*
    };
}

impl_link!(Link, VideoLink, PlaylistLink, ChannelLink, BangerLink);

#[derive(Debug, Clone, PartialEq, Eq, Hash, From)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Link {
    Video(VideoLink),
    Playlist(PlaylistLink),
    Channel(ChannelLink),
    Banger(BangerLink),
    #[from(ignore)]
    OtherPlatform(url::Url),
}

impl Link {
    pub fn from_id(id: super::ItemId<'_>) -> Self {
        match id {
            super::ItemId::VideoId(video_id) => Self::from_video_id(video_id),
            super::ItemId::BangerId(banger_id) => {
                Self::from_banger_id(banger_id)
            }
        }
    }

    pub fn from_video_id(id: &VideoId) -> Self {
        Self::Video(VideoLink::from_id(id))
    }

    pub fn from_banger_id(id: &BangerId) -> Self {
        Self::Banger(BangerLink::from_id(id))
    }

    pub fn from_playlist_id(id: &PlaylistId) -> Self {
        Self::Playlist(PlaylistLink::from_id(id))
    }

    pub fn video_id(&self) -> Option<&VideoId> {
        match self {
            Self::Video(l) => Some(l.id()),
            Self::Playlist(l) => l.video_id(),
            Self::Banger(_) | Self::Channel(_) | Self::OtherPlatform(_) => None,
        }
    }

    pub fn banger_id(&self) -> Option<&BangerId> {
        match self {
            Self::Banger(l) => Some(l.id()),
            Self::Playlist(_)
            | Self::Video(_)
            | Self::Channel(_)
            | Self::OtherPlatform(_) => None,
        }
    }

    pub fn playlist_id(&self) -> Option<&PlaylistId> {
        match self {
            Self::Playlist(l) => Some(l.id()),
            Self::Video(_)
            | Self::Banger(_)
            | Self::Channel(_)
            | Self::OtherPlatform(_) => None,
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Video(l) => l.as_str(),
            Self::Playlist(l) => l.as_str(),
            Self::Channel(l) => l.as_str(),
            Self::Banger(l) => l.as_str(),
            Self::OtherPlatform(url) => url.as_str(),
        }
    }

    pub fn into_string(self) -> String {
        match self {
            Self::Video(l) => l.into_string(),
            Self::Playlist(l) => l.into_string(),
            Self::Channel(l) => l.into_string(),
            Self::Banger(l) => l.into_string(),
            Self::OtherPlatform(url) => url.into(),
        }
    }

    pub fn as_video(&self) -> Option<&VideoLink> {
        match self {
            Self::Video(l) => Some(l),
            Self::Playlist(l) => l.as_video_link().ok(),
            Self::Channel(_) | Self::Banger(_) | Self::OtherPlatform(_) => None,
        }
    }

    pub fn as_banger(&self) -> Option<&BangerLink> {
        match self {
            Self::Banger(l) => Some(l),
            Self::Playlist(_)
            | Self::Channel(_)
            | Self::Video(_)
            | Self::OtherPlatform(_) => None,
        }
    }

    pub fn into_video(self) -> Result<VideoLink, Self> {
        match self {
            Self::Video(l) => Ok(l),
            Self::Playlist(l) => l.into_video_link().map_err(Self::Playlist),
            s @ (Self::Banger(_)
            | Self::Channel(_)
            | Self::OtherPlatform(_)) => Err(s),
        }
    }

    pub fn as_playlist(&self) -> Option<&PlaylistLink> {
        match self {
            Self::Playlist(l) => Some(l),
            Self::Video(_)
            | Self::Channel(_)
            | Self::Banger(_)
            | Self::OtherPlatform(_) => None,
        }
    }

    pub fn as_playlist_mut(&mut self) -> Option<&mut PlaylistLink> {
        match self {
            Self::Playlist(l) => Some(l),
            Self::Video(_)
            | Self::Banger(_)
            | Self::Channel(_)
            | Self::OtherPlatform(_) => None,
        }
    }

    #[cfg(all(feature = "ytdl", feature = "playlist"))]
    pub async fn runtime(&self) -> Result<Duration, crate::Error> {
        use crate::ytdl::YtdlBuilder;
        use futures_util::TryStreamExt as _;

        match self {
            Self::Video(link) => Ok(YtdlBuilder::new(link)
                .get_duration()
                .request()
                .await?
                .duration()),
            Link::Banger(banger_link) => {
                Ok(banger_link.metadata().await?.duration)
            }
            Self::Channel(_) => Ok(Duration::ZERO),
            Self::Playlist(playlist) => {
                YtdlBuilder::new(playlist)
                    .get_duration()
                    .request_playlist()
                    .await?
                    .map_ok(|i| i.duration())
                    .try_fold(Duration::ZERO, |acc, i| {
                        std::future::ready(Ok(acc + i))
                    })
                    .await
            }
            Link::OtherPlatform(_) => Ok(Duration::ZERO),
        }
    }
}

impl TryFrom<Url> for Link {
    type Error = Url;

    fn try_from(url: Url) -> Result<Self, Self::Error> {
        PlaylistLink::try_from(url)
            .map(Self::Playlist)
            .or_else(|url| BangerLink::try_from(url).map(Self::Banger))
            .or_else(|url| VideoLink::try_from(url).map(Self::Video))
            .or_else(|url| ChannelLink::try_from(url).map(Self::Channel))
            .or_else(|url| Ok(Self::OtherPlatform(url)))
    }
}

fn id_from_link<T: YtId + ?Sized>(s: &Url) -> Option<&T> {
    s.query_pairs()
        .find(|(k, _)| k == T::QUERY_PARAM)
        .map(|(_, id)| match id {
            Cow::Owned(_) => unreachable!("yt ids are always url safe"),
            Cow::Borrowed(id) => T::new(id),
        })
}

impl AsRef<str> for Link {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct VideoLink(Url);

impl VideoLink {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
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
    /// Resolve a link by trying to find it in the playlist and then querying youtube for it's
    /// title.
    #[cfg(all(feature = "ytdl", feature = "playlist"))]
    #[tracing::instrument(fields(self = self.as_str()))]
    pub async fn resolve_link(&self) -> String {
        use crate::{item::title_cache, ytdl::YtdlBuilder};

        match title_cache::get_by_vid_id(self.id()).await {
            Ok(Some(title)) => return title,
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(error = ?e, "failed to fetch from title cache");
            }
        }

        match YtdlBuilder::new(self).get_title().request().await {
            Ok(r) => {
                let title = r.title();
                if let Err(e) =
                    title_cache::put_by_vid_id(self.id(), &title).await
                {
                    tracing::warn!(error = ?e, "failed to cache title");
                }
                title
            }
            Err(e) => {
                tracing::warn!("failed to resolve link using yt dl: {e:?}");
                self.to_string()
            }
        }
    }
}

impl AsRef<str> for VideoLink {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl TryFrom<Url> for VideoLink {
    type Error = Url;

    fn try_from(s: Url) -> Result<Self, Self::Error> {
        if Self::is_video_link(&s) {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }
}

impl HasId for VideoLink {
    type Id = VideoId;
    fn id(&self) -> &Self::Id {
        id_from_link(&self.0)
            .or_else(|| VideoId::from_short_url(&self.0))
            .expect("video link to have a video id")
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

    pub fn to_link(&self) -> VideoLink {
        VideoLink::from_id(self)
    }
}

impl Deref for VideoId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for VideoId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl YtId for VideoId {
    const QUERY_PARAM: &'static str = "v";
    fn new(s: &str) -> &Self {
        unsafe { std::mem::transmute(s) }
    }

    fn boxed(&self) -> Box<Self> {
        let b: Box<str> = Box::from(self.as_str());
        unsafe { std::mem::transmute(b) }
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

    pub fn from_id(s: &PlaylistId) -> Self {
        let mut base = Url::parse("https://youtube.com/playlist").unwrap();
        base.query_pairs_mut()
            .append_pair(PlaylistId::QUERY_PARAM, &s.0);
        Self(base)
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
        VideoLink::try_from(self.0).map_err(Self)
    }

    pub fn as_video_link(&self) -> Result<&VideoLink, &Self> {
        if VideoLink::is_video_link(&self.0) {
            Ok(unsafe {
                std::mem::transmute::<&PlaylistLink, &VideoLink>(self)
            })
        } else {
            Err(self)
        }
    }
}

impl AsRef<str> for PlaylistLink {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<Url> for PlaylistLink {
    type Error = Url;
    fn try_from(s: Url) -> Result<Self, Self::Error> {
        if s.scheme().starts_with("http")
            && s.query_pairs().any(|(k, _)| k == PlaylistId::QUERY_PARAM)
        {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }
}

impl HasId for PlaylistLink {
    type Id = PlaylistId;
    fn id(&self) -> &Self::Id {
        self.0
            .query_pairs()
            .find(|(k, _)| k == PlaylistId::QUERY_PARAM)
            .map(|(_, id)| match id {
                Cow::Owned(_) => unreachable!("yt ids are always url safe"),
                Cow::Borrowed(id) => PlaylistId::new(id),
            })
            .unwrap()
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

impl AsRef<str> for PlaylistId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl YtId for PlaylistId {
    const QUERY_PARAM: &'static str = "list";
    fn new(s: &str) -> &Self {
        unsafe { std::mem::transmute(s) }
    }

    fn boxed(&self) -> Box<Self> {
        let b: Box<str> = Box::from(self.as_str());
        unsafe { std::mem::transmute(b) }
    }
}

impl Deref for PlaylistId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct ChannelLink(Url);

impl ChannelLink {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn into_string(self) -> String {
        self.0.into()
    }
}

impl AsRef<str> for ChannelLink {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl TryFrom<Url> for ChannelLink {
    type Error = Url;
    fn try_from(s: Url) -> Result<Self, Self::Error> {
        if s.scheme().starts_with("http")
            && s.path()
                .split('/')
                .nth(1)
                .is_some_and(|s| s.starts_with('@'))
        {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct BangerLink(Url);

impl BangerLink {
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn from_id(s: &BangerId) -> Self {
        Self(
            format!(
                "https://blind-eternities.mendess.xyz/playlist/song/audio/{}",
                s.as_str()
            )
            .parse()
            .unwrap(),
        )
    }

    pub fn into_string(self) -> String {
        self.0.into()
    }

    fn is_valid_link(s: &Url) -> bool {
        s.scheme().starts_with("http")
            && s.host_str().is_some_and(|host| {
                host.contains("blind-eternities.mendess.xyz")
            })
            && s.path().starts_with("/playlist/song/audio")
    }

    #[cfg(feature = "downloads")]
    pub async fn metadata(&self) -> reqwest::Result<BangerMetadata> {
        let url = {
            let id = self.id();
            let mut url = self.0.clone();
            url.set_path("/playlist/song/metadata/");
            url.join(id.as_str()).unwrap()
        };
        crate::http::client()?
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }
}

#[cfg(feature = "downloads")]
#[derive(Deserialize)]
pub struct BangerMetadata {
    pub title: String,
    pub duration: Duration,
}

impl HasId for BangerLink {
    type Id = BangerId;
    fn id(&self) -> &Self::Id {
        BangerId::from_url(&self.0).unwrap()
    }
}

impl AsRef<str> for BangerLink {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl TryFrom<Url> for BangerLink {
    type Error = Url;

    fn try_from(s: Url) -> Result<Self, Self::Error> {
        if Self::is_valid_link(&s) {
            Ok(Self(s))
        } else {
            Err(s)
        }
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct BangerId(str);

impl BangerId {
    pub fn new(s: &str) -> &Self {
        unsafe { std::mem::transmute(s) }
    }

    pub fn boxed(&self) -> Box<Self> {
        let b: Box<str> = Box::from(self.as_str());
        unsafe { std::mem::transmute(b) }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn from_url(s: &Url) -> Option<&Self> {
        BangerLink::is_valid_link(s)
            .then(|| s.path().split("/").last())
            .flatten()
            .map(|s| unsafe { std::mem::transmute(s) })
    }

    pub fn to_link(&self) -> BangerLink {
        BangerLink::from_id(self)
    }
}

impl Deref for BangerId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for BangerId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn strip_video_id() {
        const BEFORE: &str = "https://www.youtube.com/watch?v=UpIBKNxSeZU&list=PL17PSucW5L7nEPyX3tqEzq_wmyYk2IkXr";
        const AFTER: &str = "https://www.youtube.com/playlist?list=PL17PSucW5L7nEPyX3tqEzq_wmyYk2IkXr";

        let playlist_link = PlaylistLink::try_from(BEFORE.to_string())
            .unwrap()
            .without_video_id();
        assert_eq!(AFTER, playlist_link.as_str());
    }
}
