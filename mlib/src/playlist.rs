use csv::ReaderBuilder;
use dirs::config_dir;
use serde::{Deserialize, Serialize};
use std::{env, io, path::PathBuf};

#[derive(Serialize, Deserialize, Debug)]
pub struct Song {
    pub name: String,
    pub link: String,
    pub time: usize,
    #[serde(default)]
    pub categories: Vec<String>,
}

pub struct Playlist(pub Vec<Song>);

impl Playlist {
    fn path() -> Option<PathBuf> {
        env::var_os("PLAYLIST").map(PathBuf::from).or_else(|| {
            let mut playlist_path = config_dir()?;
            playlist_path.push("m");
            playlist_path.push("playlist");
            Some(playlist_path)
        })
    }

    pub fn load() -> io::Result<Self> {
        let playlist_path = Self::path().ok_or(io::ErrorKind::NotFound)?;
        let mut reader = ReaderBuilder::new()
            .delimiter(b'\t')
            .quoting(false)
            .flexible(true)
            .has_headers(false)
            .from_path(playlist_path)?;
        Ok(Self(reader.deserialize().collect::<Result<_, _>>()?))
    }
}
