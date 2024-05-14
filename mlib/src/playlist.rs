mod uniq_vec;

use csv_async::{AsyncReaderBuilder, AsyncWriterBuilder, StringRecord};
use dirs::config_dir;
use futures_util::{stream::TryStreamExt, Stream};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    env,
    fmt::{self, Display},
    io,
    ops::{Deref, DerefMut},
    path::PathBuf,
};
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncRead, AsyncReadExt},
    sync::OnceCell,
};
use tracing::debug;

use crate::{item::link::VideoLink, Error, VideoId};

#[derive(Serialize, Deserialize, Debug)]
pub struct Song {
    pub name: String,
    pub link: VideoLink,
    pub time: u64,
    #[serde(default)]
    pub categories: uniq_vec::UniqVec<String>,
}

impl Display for Song {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} :: {} :: {}", self.name, self.link, self.time)?;
        if !self.categories.is_empty() {
            write!(f, " :: ")?;
            std::iter::repeat(",")
                .zip(self.categories.iter().map(String::as_str))
                .flat_map(|(a, b)| [a, b])
                .skip(1)
                .try_for_each(|s| f.write_str(s))?;
        }
        Ok(())
    }
}

pub struct Playlist {
    pub songs: Vec<Song>,
}

static WRITER_BUILDER: Lazy<AsyncWriterBuilder> = Lazy::new(|| {
    let mut builder = AsyncWriterBuilder::new();
    builder
        .delimiter(b'\t')
        .has_headers(false)
        .flexible(true)
        .quote_style(csv_async::QuoteStyle::Never);
    builder
});

static READER_BUILDER: Lazy<AsyncReaderBuilder> = Lazy::new(|| {
    let mut reader = AsyncReaderBuilder::new();
    reader
        .delimiter(b'\t')
        .quoting(false)
        .flexible(true)
        .has_headers(false);
    reader
});

impl Playlist {
    pub(crate) fn path() -> io::Result<PathBuf> {
        thread_local! {
            static PATH: RefCell<io::Result<PathBuf>> = RefCell::new(Err(io::ErrorKind::NotFound.into()));
        };
        PATH.with(|p| {
            let mut borrow = p.borrow_mut();
            match &*borrow {
                Ok(p) => Ok(p.clone()),
                Err(_) => {
                    let path = env::var_os("PLAYLIST")
                        .map(PathBuf::from)
                        .or_else(|| {
                            let mut playlist_path = config_dir()?;
                            playlist_path.push("m");
                            playlist_path.push("playlist");
                            Some(playlist_path)
                        })
                        .ok_or(io::ErrorKind::NotFound)?;
                    *borrow = Ok(path.clone());
                    Ok(path)
                }
            }
        })
    }

    pub async fn load() -> Result<Self, Error> {
        let playlist_path = Self::path()?;
        Self::load_from(playlist_path).await
    }

    pub async fn load_from(playlist_path: PathBuf) -> Result<Self, Error> {
        let file = match File::open(&playlist_path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::PlaylistFileNotFound(playlist_path))
            }
            Err(e) => return Err(e.into()),
        };
        Self::load_from_reader(file).await
    }

    pub async fn load_from_reader<R: AsyncRead + Unpin + Send>(source: R) -> Result<Self, Error> {
        let reader = READER_BUILDER.create_deserializer(source);
        Ok(Self {
            songs: reader.into_deserialize().try_collect().await?,
        })
    }

    pub async fn stream() -> Result<impl Stream<Item = Result<Song, csv_async::Error>>, Error> {
        let playlist_path = Self::path()?;
        Self::stream_from(playlist_path).await
    }

    pub async fn stream_from(
        playlist_path: PathBuf,
    ) -> Result<impl Stream<Item = Result<Song, csv_async::Error>>, Error> {
        let file = match File::open(&playlist_path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::PlaylistFileNotFound(playlist_path))
            }
            Err(e) => return Err(e.into()),
        };
        let reader = READER_BUILDER.create_deserializer(file);
        Ok(reader.into_deserialize())
    }

    pub fn categories(&self) -> impl Iterator<Item = (&str, usize)> {
        self.songs
            .iter()
            .flat_map(|s| s.categories.iter())
            .fold(HashMap::new(), |mut set, c| {
                *set.entry(c).or_default() += 1;
                set
            })
            .into_iter()
            .map(|(k, v)| (k.as_str(), v))
    }

    pub async fn contains_song(song: &str) -> io::Result<bool> {
        let path = Playlist::path()?;
        let mut buf = Vec::new();
        File::open(&path).await?.read_to_end(&mut buf).await?;
        Ok(memchr::memmem::find(&buf, song.as_bytes()).is_some())
    }

    pub async fn add_song(song: &Song) -> Result<(), Error> {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(Self::path()?)
            .await?;
        WRITER_BUILDER
            .create_serializer(file)
            .serialize(song)
            .await
            .map_err(io::Error::from)?;
        Ok(())
    }

    pub fn find_song<F: FnMut(&Song) -> bool>(&self, f: F) -> Option<PlaylistIndex<'_>> {
        self.songs.iter().position(f).map(|index| PlaylistIndex {
            source: self,
            index,
        })
    }

    pub fn find_song_mut<F: FnMut(&mut Song) -> bool>(
        &mut self,
        f: F,
    ) -> Option<PlaylistIndexMut<'_>> {
        self.songs
            .iter_mut()
            .position(f)
            .map(|index| PlaylistIndexMut {
                source: self,
                index,
            })
    }

    pub fn partial_name_search<'s>(
        &self,
        words: impl Iterator<Item = &'s str>,
    ) -> PartialSearchResult<PlaylistIndex<'_>> {
        self.partial_name_search_impl(words)
            .map(|index| PlaylistIndex {
                source: self,
                index,
            })
    }

    pub fn partial_name_search_mut<'s>(
        &mut self,
        words: impl Iterator<Item = &'s str>,
    ) -> PartialSearchResult<PlaylistIndexMut<'_>> {
        self.partial_name_search_impl(words)
            .map(|index| PlaylistIndexMut {
                source: self,
                index,
            })
    }

    fn partial_name_search_impl<'s>(
        &self,
        words: impl Iterator<Item = &'s str>,
    ) -> PartialSearchResult<usize> {
        let mut idxs = (0..self.songs.len()).collect::<Vec<_>>();
        words.for_each(|w| {
            let regex = regex::RegexBuilder::new(&regex::escape(w))
                .case_insensitive(true)
                .build()
                .unwrap();
            idxs.retain(|i| regex.is_match(&self.songs[*i].name))
        });
        match &idxs[..] {
            [index] => PartialSearchResult::One(*index),
            [] => PartialSearchResult::None,
            many => PartialSearchResult::Many(
                many.iter().map(|i| self.songs[*i].name.clone()).collect(),
            ),
        }
    }

    pub async fn save(&self) -> Result<(), Error> {
        let file = File::create(Self::path()?).await?;
        let mut writer = WRITER_BUILDER.create_serializer(file);
        for song in self.songs.iter() {
            writer.serialize(song).await?;
        }
        Ok(())
    }

    pub fn find_by_link(&self, link: &VideoLink) -> Option<&Song> {
        self.songs.iter().find(|s| s.link.id() == link.id())
    }
}

pub enum PartialSearchResult<T> {
    None,
    One(T),
    Many(Vec<String>),
}

impl<T> PartialSearchResult<T> {
    #[inline(always)]
    fn map<R>(self, f: impl FnOnce(T) -> R) -> PartialSearchResult<R> {
        match self {
            PartialSearchResult::One(t) => PartialSearchResult::One(f(t)),
            PartialSearchResult::None => PartialSearchResult::None,
            PartialSearchResult::Many(x) => PartialSearchResult::Many(x),
        }
    }
}

impl<T> From<Option<T>> for PartialSearchResult<T> {
    fn from(o: Option<T>) -> Self {
        match o {
            Some(t) => PartialSearchResult::One(t),
            None => PartialSearchResult::None,
        }
    }
}

pub struct PlaylistIndex<'p> {
    source: &'p Playlist,
    index: usize,
}

impl Deref for PlaylistIndex<'_> {
    type Target = Song;

    fn deref(&self) -> &Self::Target {
        &self.source.songs[self.index]
    }
}

pub struct PlaylistIndexMut<'p> {
    source: &'p mut Playlist,
    index: usize,
}

impl Deref for PlaylistIndexMut<'_> {
    type Target = Song;

    fn deref(&self) -> &Self::Target {
        &self.source.songs[self.index]
    }
}

impl DerefMut for PlaylistIndexMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.source.songs[self.index]
    }
}

impl PlaylistIndexMut<'_> {
    pub fn delete(self) -> Song {
        self.source.songs.remove(self.index)
    }
}

pub async fn find_song(id: &VideoId) -> Result<Option<Song>, Error> {
    let path = Playlist::path()?;
    let mut buf = Vec::new();
    File::open(&path).await?.read_to_end(&mut buf).await?;
    match memchr::memmem::find(&buf, id.as_bytes()) {
        Some(i) => {
            let end = memchr::memmem::find(&buf[i..], b"\n")
                .map(|new_line| new_line + i)
                .unwrap_or_else(|| buf.len());
            let start = memchr::memmem::rfind(&buf[..i], b"\n")
                .map(|i| i + 1)
                .unwrap_or(0);
            let mut fields = buf[start..end]
                .split(|c| *c == b'\t')
                .map(<[u8]>::to_vec)
                .map(String::from_utf8)
                .map(Result::unwrap);
            let mut next_field = || {
                fields
                    .next()
                    .ok_or_else(|| Error::PlaylistFile(String::from("not enough fields")))
            };
            Ok(Some(Song {
                name: next_field()?,
                link: next_field()?
                    .try_into()
                    .map_err(|e| Error::PlaylistFile(format!("invalid link: {e}")))?,
                time: next_field()?
                    .parse()
                    .map_err(|_| Error::PlaylistFile("invalid duration".into()))?,
                categories: fields.collect(),
            }))
        }
        None => Ok(None),
    }
}

pub struct PlaylistIds(HashSet<String>);

impl PlaylistIds {
    pub async fn load() -> io::Result<Self> {
        let playlist_path = Playlist::path()?;
        let file = File::open(playlist_path).await?;
        let mut reader = READER_BUILDER.create_deserializer(file);
        let mut record = StringRecord::new();
        let mut set = HashSet::new();
        while reader.read_record(&mut record).await? {
            //TODO: unwrap
            let id = record.get(1).unwrap().split('/').last().unwrap();
            set.insert(id.to_string());
        }
        Ok(Self(set))
    }

    pub fn contains(&self, l: &str) -> bool {
        self.0.contains(l)
    }
}

impl VideoLink {
    /// Resolve a link by trying to find it in the playlist and then querying youtube for it's
    /// title.
    pub async fn resolve_link(&self) -> String {
        static LIST: OnceCell<Result<Playlist, crate::Error>> = OnceCell::const_new();
        debug!("resolving link in playlist");
        let name = match LIST.get_or_init(Playlist::load).await {
            Ok(list) => Ok(list.find_by_link(self).map(|s| s.name.clone())),
            Err(e) => Err(e),
        };
        match name {
            Ok(Some(name)) => name,
            #[cfg(feature = "ytdl")]
            e => {
                debug!("failed to find link in playlist: {e:?}");
                use crate::ytdl::YtdlBuilder;
                match YtdlBuilder::new(self).get_title().request().await {
                    Ok(r) => r.title(),
                    Err(e) => {
                        tracing::warn!("failed to resolve link using yt dl: {e:?}");
                        self.to_string()
                    }
                }
            }
            #[cfg(not(feature = "ytdl"))]
            e => {
                debug!("failed to find link in playlist: {e:?}");
                self.to_string()
            }
        }
    }
}
