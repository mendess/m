use csv_async::{AsyncReaderBuilder, AsyncWriterBuilder, StringRecord};
use dirs::config_dir;
use futures_util::stream::TryStreamExt;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    env,
    fmt::{self, Display},
    io,
    path::PathBuf,
};
use tokio::{
    fs::{File, OpenOptions},
    io::AsyncReadExt,
};

use crate::Error;

#[derive(Serialize, Deserialize, Debug)]
pub struct Song {
    pub name: String,
    pub link: String,
    pub time: u64,
    #[serde(default)]
    pub categories: Vec<String>,
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
    builder.delimiter(b'\t').has_headers(false);
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

    pub async fn load() -> io::Result<Self> {
        let playlist_path = Self::path()?;
        let file = File::open(playlist_path).await?;
        let mut reader = READER_BUILDER.create_deserializer(file);
        Ok(Self(reader.deserialize().try_collect().await?))
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

    pub async fn find_song(id: &str) -> Result<Option<Song>, Error> {
        let path = Playlist::path()?;
        let mut buf = Vec::new();
        File::open(&path).await?.read_to_end(&mut buf).await?;
        match memchr::memmem::find(&buf, id.as_bytes()) {
            Some(i) => {
                let end = memchr::memmem::find(&buf[i..], b"\n").unwrap_or(buf.len());
                let start = memchr::memmem::rfind(&buf[..i], b"\n")
                    .map(|i| i + 1)
                    .unwrap_or(0);
                println!("{}", std::str::from_utf8(&buf[start..end]).unwrap());
                let mut i = buf[start..end]
                    .split(|c| *c == b'\t')
                    .map(<[u8]>::to_vec)
                    .map(String::from_utf8)
                    .map(Result::unwrap);
                Ok((|| {
                    Some(Song {
                        name: i.next()?,
                        link: i.next()?,
                        time: i.next()?.parse().unwrap(), // TODO: fix this shit
                        categories: i.collect(),
                    })
                })())
            }
            None => Ok(None),
        }
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
