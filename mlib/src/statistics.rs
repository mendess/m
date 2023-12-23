// # times a song was skipped
// # times a song was dequeued
// # times a song was played
// # times a category was queued
// # times a category was unqueued

use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufReader, BufWriter},
    path::{Path, PathBuf},
};

use chrono::Datelike;
use raii_flock::FileLock;
use serde::{Deserialize, Serialize};
use serde_map_to_array::HashMapToArray;
use tempfile::NamedTempFile;

use crate::Item;

#[derive(Default, Debug, Clone, Copy, Serialize, Deserialize)]
struct SongStats {
    played: u64,
    skipped: u64,
    dequeued: u64,
}

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
struct Stats {
    #[serde(with = "HashMapToArray::<Item, SongStats>")]
    songs: HashMap<Item, SongStats>,
}

async fn update_db<F>(f: F) -> io::Result<()>
where
    F: FnOnce(&mut Stats) + Send + 'static,
{
    async fn path() -> io::Result<PathBuf> {
        let Some(mut stats_path) = dirs::data_dir() else {
            tracing::error!("failed to get data dir for stat tracking");
            return Err(io::ErrorKind::NotFound.into());
        };

        let current_year = chrono::Utc::now().date_naive().year();
        stats_path.push("m");
        tokio::fs::create_dir_all(&stats_path).await?;
        stats_path.push(format!("statistics-{current_year}.json"));
        Ok(stats_path)
    }

    fn load_db(stats_file: &File) -> io::Result<Stats> {
        let reader = BufReader::new(stats_file);
        Ok(serde_json::from_reader(reader)?)
    }
    fn store_db(stats_path: &Path, stats: Stats) -> io::Result<()> {
        let dir = stats_path.parent().unwrap();
        let (file, temp_path) = NamedTempFile::new_in(dir)?.into_parts();
        let writer = BufWriter::new(file);
        serde_json::to_writer(writer, &stats)?;

        std::fs::rename(&temp_path, stats_path)?;

        Ok(())
    }
    let stats_path = path().await?;
    tokio::task::spawn_blocking(move || {
        let file;
        let (_file_lock, mut stats) = match File::open(&stats_path) {
            Ok(f) => {
                file = f;
                (FileLock::wrap_exclusive(&file), load_db(&file)?)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                file = File::create(&stats_path)?;
                (FileLock::wrap_exclusive(&file), Stats::default())
            }
            Err(e) => return Err(e),
        };
        f(&mut stats);
        store_db(&stats_path, stats)
    })
    .await?
}

pub async fn played_song(item: Item) -> io::Result<()> {
    update_db(|stats| {
        stats.songs.entry(item).or_default().played += 1;
    })
    .await
}

pub async fn skipped_song(item: Item) -> io::Result<()> {
    update_db(|stats| {
        stats.songs.entry(item).or_default().skipped += 1;
    })
    .await
}

pub async fn dequeued_song(item: Item) -> io::Result<()> {
    update_db(|stats| {
        stats.songs.entry(item).or_default().dequeued += 1;
    })
    .await
}
