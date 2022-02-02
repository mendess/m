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
    io::AsyncReadExt,
};

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

pub struct Playlist(pub Vec<Song>);

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
        let file = match File::open(&playlist_path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::PlaylistFileNotFound(playlist_path))
            },
            Err(e) => return Err(e.into())
        };
        let reader = READER_BUILDER.create_deserializer(file);
        Ok(Self(reader.into_deserialize().try_collect().await?))
    }

    pub async fn stream() -> Result<impl Stream<Item = Result<Song, csv_async::Error>>, Error> {
        let playlist_path = Self::path()?;
        let file = match File::open(&playlist_path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::PlaylistFileNotFound(playlist_path))
            },
            Err(e) => return Err(e.into())
        };
        let reader = READER_BUILDER.create_deserializer(file);
        Ok(reader.into_deserialize())
    }

    pub fn categories(&self) -> impl Iterator<Item = (&str, usize)> {
        self.0
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
        self.0.iter().position(f).map(|index| PlaylistIndex {
            source: self,
            index,
        })
    }

    pub fn find_song_mut<F: FnMut(&mut Song) -> bool>(
        &mut self,
        f: F,
    ) -> Option<PlaylistIndexMut<'_>> {
        self.0.iter_mut().position(f).map(|index| PlaylistIndexMut {
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
        let mut idxs = (0..self.0.len()).collect::<Vec<_>>();
        words.for_each(|w| {
            let regex = regex::RegexBuilder::new(&regex::escape(w))
                .case_insensitive(true)
                .build()
                .unwrap();
            idxs.retain(|i| regex.is_match(&self.0[*i].name))
        });
        match &idxs[..] {
            [index] => PartialSearchResult::One(*index),
            [] => PartialSearchResult::None,
            many => {
                PartialSearchResult::Many(many.iter().map(|i| self.0[*i].name.clone()).collect())
            }
        }
    }

    pub async fn save(&self) -> Result<(), Error> {
        let file = File::create(Self::path()?).await?;
        let mut writer = WRITER_BUILDER.create_serializer(file);
        for s in self.0.iter() {
            writer.serialize(s).await.map_err(Error::from)?;
        }
        Ok(())
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
        &self.source.0[self.index]
    }
}

pub struct PlaylistIndexMut<'p> {
    source: &'p mut Playlist,
    index: usize,
}

impl Deref for PlaylistIndexMut<'_> {
    type Target = Song;

    fn deref(&self) -> &Self::Target {
        &self.source.0[self.index]
    }
}

impl DerefMut for PlaylistIndexMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.source.0[self.index]
    }
}

impl PlaylistIndexMut<'_> {
    pub fn delete(self) -> Song {
        self.source.0.remove(self.index)
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
                .unwrap_or_else(|| dbg!(buf.len()));
            let start = memchr::memmem::rfind(&buf[..i], b"\n")
                .map(|i| i + 1)
                .unwrap_or(0);
            let mut i = buf[start..end]
                .split(|c| *c == b'\t')
                .map(<[u8]>::to_vec)
                .map(String::from_utf8)
                .map(Result::unwrap);
            let not_enough_fields = || Error::PlaylistFile(String::from("not enough fields"));
            Ok(Some(Song {
                name: i.next().ok_or_else(not_enough_fields)?,
                link: VideoLink::from_url(i.next().ok_or_else(not_enough_fields)?)
                    .map_err(|a| Error::PlaylistFile(format!("invalid link: {}", a)))?,
                time: i
                    .next()
                    .and_then(|n| n.parse().ok())
                    .ok_or_else(|| Error::PlaylistFile("invalid duration".into()))?,
                categories: i.collect(),
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
