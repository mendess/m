use crate::{
    downloaded::download,
    item::{link::Id, VideoLink},
    players::daemon::player::MpvExt,
    VideoId,
};
use libmpv::{FileState, Mpv};
use parking_lot::Mutex;
use std::{collections::HashMap, path::Path, sync::Weak, time::Duration};
use tokio::sync::{oneshot, Semaphore};

pub struct Task {
    cancel: Option<oneshot::Sender<()>>,
}

#[tracing::instrument(skip_all, fields(%song))]
async fn do_it(cache_dir: &Path, song: &VideoLink, player: Weak<Mpv>) {
    let path = {
        static CONCURRENT_DOWNLOADS: Semaphore = Semaphore::const_new(4);
        let _permit = CONCURRENT_DOWNLOADS.acquire().await;
        match download(cache_dir.join("m").join("preemptive-dl"), song, false).await {
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
        break;
    }
}

impl Task {
    fn new(id: &VideoLink, player: Weak<Mpv>) -> Self {
        let (tx, rx) = oneshot::channel();
        let song = id.clone();
        tokio::spawn(async move {
            let Some(cache_dir) = dirs::cache_dir() else {
                tracing::warn!(
                    %song,
                    "cache dir not present, not preemptively downloading song"
                );
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

impl PreemptiveDownload {
    pub fn new(player: Weak<Mpv>) -> Self {
        Self {
            player,
            inflight: Default::default(),
        }
    }

    pub fn song_queued(&self, item: &VideoLink) {
        self.inflight
            .lock()
            .insert(item.id().boxed(), Task::new(item, self.player.clone()));
    }

    pub fn song_dequeued(&self, item: &VideoLink) {
        self.inflight.lock().remove(item.id());
    }

    pub fn stop_all(&self) {
        self.inflight.lock().clear();
    }
}
