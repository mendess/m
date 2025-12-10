use crate::{
    Item, Link, VideoId,
    downloaded::yt_download,
    item::{
        VideoLink,
        link::{HasId as _, YtId},
    },
    players::daemon::player::MpvExt,
};
use libmpv::{FileState, Mpv};
use parking_lot::Mutex;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Weak,
    time::Duration,
};
use tokio::sync::{Semaphore, oneshot};

pub struct Task {
    cancel: Option<oneshot::Sender<()>>,
}

fn cache_dir() -> Option<PathBuf> {
    let Some(mut cache_dir) = dirs::cache_dir() else {
        tracing::warn!("cache dir not present, not preemptively downloading song");
        return None;
    };
    cache_dir.push("m");
    cache_dir.push("preemptive-dl");
    Some(cache_dir)
}

#[tracing::instrument(skip_all, fields(%song))]
async fn do_it(dl_dir: &Path, song: &VideoLink, player: Weak<Mpv>) {
    let path = {
        static CONCURRENT_DOWNLOADS: Semaphore = Semaphore::const_new(4);
        let _permit = CONCURRENT_DOWNLOADS.acquire().await;
        match yt_download(dl_dir.to_path_buf(), song, false).await {
            Ok(path) => match path.get().await {
                Ok(path) => path,
                Err(e) => {
                    tracing::error!(error = ?e, "failed to get file path for downloaded song");
                    return;
                }
            },
            Err(e) => {
                tracing::error!(error = ?e, "failed to preemptively download song");
                return;
            }
        }
    };
    loop {
        let Some(player) = player.upgrade() else {
            return;
        };
        static UPDATE_QUEUE_SEMAPHORE: Semaphore = Semaphore::const_new(1);
        let permit = UPDATE_QUEUE_SEMAPHORE.acquire().await;

        let Ok(from) = player.simple_prop::<i64>("playlist-count") else {
            return;
        };
        let Some(from) = usize::try_from(from).ok().and_then(|f| f.checked_sub(1)) else {
            return;
        };
        tracing::debug!(?from);
        let to = {
            let Ok(playlist) = player.playlist() else {
                return;
            };
            let position = playlist.into_iter().enumerate().find_map(|(pos, item)| {
                item.as_ref()
                    .is_ok_and(|i| i.filename == song.as_str())
                    .then_some(pos)
            });
            match position {
                Some(position) => position,
                None => return,
            }
        };
        tracing::debug!(?to);
        let Ok(current_pos) = player.simple_prop::<i64>("playlist-pos") else {
            return;
        };
        tracing::debug!(?current_pos);
        if to == current_pos as usize {
            tracing::debug!("playing this song right now. Waiting for a chance to replace");
            drop(permit);
            drop(player);
            tokio::time::sleep(Duration::from_secs(60)).await;
            continue;
        }
        tracing::debug!("queueing cached version");
        if let Err(e) =
            player.playlist_load_files(&[(path.to_str().unwrap(), FileState::AppendPlay, None)])
        {
            tracing::error!(error = ?e, "failed to load the downloaded version");
            return;
        };
        tracing::debug!(to, "removing old song");
        if let Err(e) = player.playlist_remove_index(to) {
            tracing::error!(error = ?e, "failed to remove the uncached version");
            return;
        };
        tracing::debug!(from, to, "moving new one");
        if let Err(e) = player.playlist_move_fixed(from as _, to) {
            tracing::error!(error = ?e, "failed to move the cached version to the position the uncached one");
            return;
        };
    }
}

impl Task {
    #[tracing::instrument(fields(%id))]
    fn new(id: &VideoLink, player: Weak<Mpv>) -> Self {
        let (tx, rx) = oneshot::channel();
        let song = id.clone();
        tokio::spawn(async move {
            let Some(cache_dir) = cache_dir() else {
                return;
            };
            let dl = do_it(&cache_dir, &song, player);
            tokio::select! {
                _ = dl => {}
                _ = rx => {}
            }
        });
        Self { cancel: Some(tx) }
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        let Some(cancel) = self.cancel.take() else {
            return;
        };
        let _ = cancel.send(());
    }
}

pub struct PreemptiveDownload {
    player: Weak<Mpv>,
    inflight: Mutex<HashMap<Box<VideoId>, Task>>,
}

fn check_cache(vid: &VideoId) -> Option<PathBuf> {
    let cache_dir = cache_dir()?;
    let cached = glob::glob(&format!("{}/*{}*", cache_dir.to_str()?, vid.as_str()))
        .ok()?
        .next()?
        .ok()?;
    Some(cached)
}

impl PreemptiveDownload {
    pub fn new(player: Weak<Mpv>) -> Self {
        Self {
            player,
            inflight: Default::default(),
        }
    }

    #[must_use]
    pub fn song_queued(&self, item: Item) -> Item {
        match &item {
            Item::Link(Link::Video(video_id)) => {
                if let Some(cached) = check_cache(video_id.id()) {
                    return Item::File(cached);
                }
                self.inflight.lock().insert(
                    video_id.id().boxed(),
                    Task::new(video_id, self.player.clone()),
                );
            }
            Item::File(_) => {}
            i => {
                tracing::warn!(item = ?i, "ignoring")
            }
        }
        item
    }

    pub fn song_dequeued(&self, item: &VideoLink) {
        self.inflight.lock().remove(item.id());
    }

    pub fn stop_all(&self) {
        self.inflight.lock().clear();
    }
}
