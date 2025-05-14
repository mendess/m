pub mod uniq_vec;

use csv_async::AsyncReaderBuilder;
use dirs::config_dir;
use futures_util::{
    Stream, StreamExt,
    stream::{self, TryStreamExt},
};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    env,
    fmt::{self, Display},
    io::{self, Write as _},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
    sync::LazyLock,
};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt as _, AsyncWriteExt as _, BufReader},
};

use crate::{Error, VideoId, item::link::VideoLink};

#[derive(Serialize, Deserialize, Debug)]
pub struct Song {
    pub name: String,
    pub link: VideoLink,
    pub time: u64,
    #[serde(default)]
    pub categories: uniq_vec::UniqVec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub genre: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recomended_by: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub liked_by: Vec<String>,
}

impl Display for Song {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} :: {} :: {}", self.name, self.link, self.time)?;
        let mut all_cat = self.all_categories().peekable();
        if all_cat.peek().is_some() {
            write!(f, " :: ")?;
            std::iter::repeat(",")
                .zip(all_cat)
                .flat_map(|(a, b)| [a, b])
                .skip(1)
                .try_for_each(|s| f.write_str(s))?;
        }
        Ok(())
    }
}

impl Song {
    pub fn all_categories(&self) -> impl Iterator<Item = &str> + Clone {
        let Song {
            name: _,
            link: _,
            time: _,
            categories,
            genre,
            artist,
            language,
            recomended_by,
            liked_by,
        } = self;
        categories
            .iter()
            .chain(artist)
            .chain(genre)
            .chain(language)
            .chain(recomended_by)
            .chain(liked_by)
            .map(|s| s.as_str())
    }
}

pub struct Playlist {
    pub songs: Vec<Song>,
}

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
    fn path() -> io::Result<&'static PathBuf> {
        static PATH: LazyLock<io::Result<PathBuf>> = LazyLock::new(|| {
            let mut path = env::var_os("PLAYLIST")
                .map(PathBuf::from)
                .or_else(|| {
                    let mut playlist_path = config_dir()?;
                    playlist_path.push("m");
                    playlist_path.push("playlist");
                    Some(playlist_path)
                })
                .ok_or(io::ErrorKind::NotFound)?;
            path.set_extension("json");
            Ok(path)
        });
        PATH.as_ref().map_err(|e| {
            tracing::error!(error = ?e, "failed to find playlist path");
            io::ErrorKind::NotFound.into()
        })
    }

    pub async fn load() -> Result<Self, Error> {
        let playlist_path = Self::path()?;
        Self::load_from(playlist_path).await
    }

    pub async fn load_from(playlist_path: &Path) -> Result<Self, Error> {
        let file = match File::open(&playlist_path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Self::legacy_load_from(&playlist_path.with_extension("")).await;
            }
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            songs: serde_json::from_reader(file.into_std().await)?,
        })
    }

    async fn legacy_load_from(playlist_path: &Path) -> Result<Self, Error> {
        let file = match File::open(&playlist_path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::PlaylistFileNotFound(playlist_path.to_owned()));
            }
            Err(e) => return Err(e.into()),
        };
        let reader = READER_BUILDER.create_deserializer(file);
        Ok(Self {
            songs: reader.into_deserialize().try_collect().await?,
        })
    }

    pub async fn stream() -> Result<impl Stream<Item = Result<Song, csv_async::Error>>, Error> {
        let playlist_path = Self::path()?;
        Self::stream_from(playlist_path).await
    }

    async fn stream_from(
        playlist_path: &Path,
    ) -> Result<impl Stream<Item = Result<Song, csv_async::Error>>, Error> {
        let file = match File::open(&playlist_path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Self::legacy_stream_from(playlist_path.with_extension(""))
                    .await
                    .map(|s| s.boxed());
            }
            Err(e) => return Err(e.into()),
        };
        Ok(stream::iter(
            serde_json::from_reader::<_, Vec<Song>>(file.into_std().await)?
                .into_iter()
                .map(Ok),
        )
        .boxed())
    }

    async fn legacy_stream_from(
        playlist_path: PathBuf,
    ) -> Result<impl Stream<Item = Result<Song, csv_async::Error>>, Error> {
        let file = match File::open(&playlist_path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(Error::PlaylistFileNotFound(playlist_path.to_owned()));
            }
            Err(e) => return Err(e.into()),
        };
        let reader = READER_BUILDER.create_deserializer(file);
        Ok(reader.into_deserialize())
    }

    pub fn categories(&self) -> HashMap<&str, usize> {
        self.songs
            .iter()
            .flat_map(|s| s.all_categories())
            .fold(HashMap::new(), |mut set, c| {
                *set.entry(c).or_default() += 1;
                set
            })
    }

    pub async fn contains_song(song: &str) -> io::Result<bool> {
        let path = Playlist::path()?;
        let mut buf = Vec::new();
        match File::open(&path).await {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                File::open(path.with_extension("")).await?
            }
            Err(e) => return Err(e),
        }
        .read_to_end(&mut buf)
        .await?;
        Ok(memchr::memmem::find(&buf, song.as_bytes()).is_some())
    }

    pub async fn add_song(song: &Song) -> Result<(), Error> {
        let path = Self::path()?;
        let mut file = match File::options().write(true).open(&path).await {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let playlist = Self::legacy_load_from(&path.with_extension("")).await?;
                playlist.save().await?;
                File::options().write(true).open(&path).await?
            }
            Err(e) => return Err(e.into()),
        };

        struct Indented<W: std::io::Write>(W);
        impl<W: std::io::Write> std::io::Write for Indented<W> {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                let mut count = 0;
                for b in buf.split_inclusive(|b| *b == b'\n') {
                    self.0.write_all(b)?;
                    count += b.len();
                    const INDENT: &[u8] = b"  ";
                    if b.ends_with(b"\n") {
                        self.0.write_all(INDENT)?;
                    }
                }
                Ok(count)
            }

            fn flush(&mut self) -> io::Result<()> {
                self.0.flush()
            }
        }
        file.seek(io::SeekFrom::End(-2)).await?;
        file.write_all(b",\n  ").await?;
        let mut std_file = file.into_std().await;
        serde_json::to_writer_pretty(Indented(&mut std_file), song)?;
        std_file.write_all(b"\n]")?;
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
        serde_json::to_writer_pretty(file.into_std().await, &self.songs)?;
        Ok(())
    }

    pub fn find_by_link(&self, link: &VideoLink) -> Option<&Song> {
        self.find_by_id(link.id())
    }

    pub fn find_by_id(&self, id: &VideoId) -> Option<&Song> {
        self.songs.iter().find(|s| s.link.id() == id)
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

#[derive(Debug)]
pub struct PlaylistIds(HashSet<String>);

impl PlaylistIds {
    pub async fn load() -> io::Result<Self> {
        let playlist_path = Playlist::path()?;
        let file = File::open(playlist_path).await?;
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        let mut set = HashSet::new();
        while {
            line.clear();
            reader.read_line(&mut line).await? > 0
        } {
            const LINK_FIELD: &str = r#""link": ""#;
            if let Some(idx) = line.find(LINK_FIELD) {
                let line = &line[(idx + LINK_FIELD.len())..];
                if let Some(end) = line.find('"') {
                    //TODO: unwrap
                    set.insert(line[..end].split('/').next_back().unwrap().to_string());
                }
            }
        }
        Ok(Self(set))
    }

    pub fn contains(&self, l: &str) -> bool {
        self.0.contains(l)
    }
}
