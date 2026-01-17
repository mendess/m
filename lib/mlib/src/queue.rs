use std::time::Duration;

pub use crate::Item;
use crate::{
    Error, Link,
    item::id_from_path,
    players::{PlayerLink, QueueItem},
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct SongIdent {
    pub index: usize,
    pub item: Item,
}

pub struct Queue {
    items: Vec<SongIdent>,
    current_idx: usize,
    pub playing: bool,
    pub last_queue: Option<usize>,
}

impl Queue {
    pub fn before(&self) -> &[SongIdent] {
        &self.items[..self.current_idx]
    }

    pub fn after(&self) -> &[SongIdent] {
        self.items.get(self.current_idx + 1..).unwrap_or_default()
    }

    pub fn current_song(&self) -> &SongIdent {
        &self.items[self.current_idx]
    }

    pub fn current_idx(&self) -> usize {
        self.items[self.current_idx].index
    }

    pub async fn load_full(player: &PlayerLink) -> Result<Self, Error> {
        Self::load(player, usize::MAX).await
    }

    pub async fn load(player: &PlayerLink, at_most: usize) -> Result<Self, Error> {
        let queue = player.queue().await?;
        let last_queue = player.last_queue().await?;
        let (items, current_idx, playing) = slice_queue(queue, at_most);
        Ok(Self {
            items,
            current_idx,
            playing,
            last_queue,
        })
    }

    pub async fn link(player: &PlayerLink) -> Result<Item, Error> {
        let current_idx = player.queue_pos().await?;
        let current = player.queue_at(current_idx).await?;
        match Item::from(current.filename) {
            Item::Link(l) => Ok(Item::Link(l)),
            Item::File(p) => Ok(id_from_path(&p)
                .map(Link::from_id)
                .map(Item::Link)
                .unwrap_or_else(|| Item::File(p))),
            Item::Search(s) => Ok(Item::Search(s)),
        }
    }

    #[tracing::instrument(skip(player))]
    #[cfg(feature = "ytdl")]
    pub async fn current(player: &PlayerLink, opt: CurrentOptions) -> Result<Current, Error> {
        pub use crate::Item;
        use crate::{
            Error,
            players::error::{Error as PlayerError, MpvError, MpvErrorCode},
            playlist,
        };

        use tracing::Instrument;

        tracing::trace!("getting current");
        let metadata = async {
            tracing::trace!("getting");
            let media_title = player.media_title().await?;
            let item = Item::from(player.filename().await?);
            let id = item.id();
            // TODO: this is wrong
            let title = if media_title.is_empty() {
                "unknown".into()
            } else {
                media_title
            };

            let playing = !player.is_paused().await?;
            let volume = player.volume().await?;
            let progress = match player.percent_position().await {
                Ok(progress) => Some(progress),
                Err(PlayerError::Mpv(MpvError::Raw(MpvErrorCode::PropertyUnavailable))) => None,
                Err(e) => return Err(e.into()),
            };
            let playback_time = match player.playback_time().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(%e, "getting the playback_time");
                    0.0
                }
            };
            let duration = match player.duration().await {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!(%e, "getting the duration");
                    0.0
                }
            };
            let playlist = playlist::Playlist::load().await?;
            let (artist, categories) = match id {
                Some(id) => (match id {
                    crate::item::ItemId::VideoId(_) => None,
                    crate::item::ItemId::BangerId(banger_id) => Some(banger_id),
                })
                .and_then(|id| playlist.find_by_id(id))
                .map(|s| {
                    (
                        s.artist.clone(),
                        s.all_categories().map(|s| s.to_owned()).collect::<Vec<_>>(),
                    )
                })
                .unwrap_or_default(),
                None => (
                    item.fetch_item_artist(&playlist)
                        .await
                        .map(|a| a.into_owned()),
                    vec![],
                ),
            };

            let chapter = player
                .chapter_metadata()
                .await
                .ok()
                .flatten()
                .map(|m| (m.index, m.title));

            tracing::trace!("done");
            Ok((
                title,
                playing,
                volume,
                progress,
                playback_time,
                duration,
                categories,
                chapter,
                artist,
            ))
        }
        .instrument(tracing::trace_span!("metadata"));

        let next = async {
            tracing::trace!("getting");
            let current_idx = player.queue_pos().await?;
            let next = match opt {
                CurrentOptions::GetNext => Self::up_next(player, current_idx).await?,
                CurrentOptions::None => None,
            };
            tracing::trace!("done");
            Ok::<_, Error>((current_idx, next))
        }
        .instrument(tracing::trace_span!("up next"));

        let (
            (current_idx, next),
            (
                title,
                playing,
                volume,
                progress,
                playback_time,
                duration,
                categories,
                chapter,
                artist,
            ),
        ) = futures_util::try_join!(next, metadata)?;

        Ok(Current {
            title,
            chapter,
            artist,
            playing,
            categories,
            volume,
            progress,
            duration: Duration::from_secs_f64(duration),
            playback_time: Duration::try_from_secs_f64(playback_time).ok(),
            index: current_idx,
            next,
        })
    }

    #[tracing::instrument(skip(player))]
    #[cfg(feature = "ytdl")]
    pub async fn up_next<I>(player: &PlayerLink, queue_index: I) -> Result<Option<UpNext>, Error>
    where
        I: Into<Option<usize>> + std::fmt::Debug,
    {
        use crate::{item::link::HasId, playlist::Playlist};
        use std::borrow::Cow;

        tracing::trace!("getting queue_size");
        let size = player.queue_size().await?;
        if size == 1 {
            return Ok(None);
        }
        let queue_index = match queue_index.into() {
            Some(idx) => idx,
            None => {
                tracing::trace!("getting queue_pos");
                player.queue_pos().await?
            }
        };
        tracing::trace!("getting queue_at");
        let next = player.queue_at((queue_index + 1) % size).await?.filename;
        let item = Item::from(next);
        let playlist = Playlist::load().await?;
        let next = UpNext {
            title: item.fetch_item_title(&playlist).await.into_owned(),
            artist: item.fetch_item_artist(&playlist).await.map(Cow::into_owned),
            categories: if let Item::Link(Link::Banger(l)) = item {
                playlist
                    .find_by_id(l.id())
                    .map(|s| s.all_categories().map(|s| s.to_owned()).collect())
                    .unwrap_or_default()
            } else {
                vec![]
            },
        };
        Ok(Some(next))
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &SongIdent> {
        self.items.iter()
    }

    pub fn for_each<F: FnMut(&SongIdent), C: FnOnce(&SongIdent)>(&self, mut f: F, c: C) {
        for i in self.before() {
            f(i)
        }
        c(self.current_song());
        for i in self.after() {
            f(i)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpNext {
    pub title: String,
    pub artist: Option<String>,
    pub categories: Vec<String>,
}

impl IntoIterator for Queue {
    type Item = SongIdent;
    type IntoIter = std::vec::IntoIter<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Copy, Default)]
pub enum CurrentOptions {
    GetNext,
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Current {
    pub title: String,
    pub chapter: Option<(usize, String)>,
    pub artist: Option<String>,
    pub playing: bool,
    pub volume: f64,
    pub progress: Option<f64>,
    pub playback_time: Option<Duration>,
    pub duration: Duration,
    pub categories: Vec<String>,
    pub index: usize,
    pub next: Option<UpNext>,
}

fn slice_queue(mut queue: Vec<QueueItem>, at_most: usize) -> (Vec<SongIdent>, usize, bool) {
    let Some((mut current_idx, st)) = queue
        .iter()
        .enumerate()
        .find_map(|(idx, item)| item.status.map(|st| (idx, st)))
    else {
        return (vec![], 0, false);
    };

    let mut start_index = current_idx.saturating_sub(at_most / 5);

    // start index is the new base, so current_idx has to become
    // relative to that base.
    current_idx -= start_index;

    let mut end_index = start_index.saturating_add(at_most);
    if end_index > queue.len() {
        let delta = end_index - queue.len();
        end_index = queue.len();
        let new_start_index = start_index.saturating_sub(delta);
        current_idx += start_index - new_start_index;
        start_index = new_start_index;
    }

    let mut next_index = start_index;
    let mut next_index = || {
        let i = next_index;
        next_index += 1;
        i
    };
    let items = queue
        .drain(start_index..end_index)
        .map(|i| SongIdent {
            index: next_index(),
            item: Item::from(i.filename),
        })
        .collect();
    (items, current_idx, st.playing)
}
