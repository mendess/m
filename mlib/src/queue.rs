use std::time::Duration;

pub use crate::Item;
use crate::{
    item::id_from_path,
    players::{PlayerLink, QueueItem},
    Error, Link,
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
                .map(Link::from_video_id)
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
            players::error::{Error as PlayerError, MpvError, MpvErrorCode},
            playlist, Error,
        };

        use futures_util::future::OptionFuture;
        use tracing::Instrument;

        tracing::trace!("getting current");
        let metadata = async {
            tracing::trace!("getting");
            let media_title = player.media_title().await?;
            let filename = Item::from(player.filename().await?);
            let id = filename.id();
            // TODO: this is wrong
            let title = if media_title.is_empty() {
                filename.to_string()
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
            let categories = OptionFuture::from(id.map(playlist::find_song))
                .await
                .transpose()?
                .flatten()
                .map(|s| s.categories)
                .unwrap_or_default();

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
            (title, playing, volume, progress, playback_time, duration, categories, chapter),
        ) = futures_util::try_join!(next, metadata)?;

        Ok(Current {
            title,
            chapter,
            playing,
            categories: categories.into_vec(),
            volume,
            progress,
            duration: Duration::from_secs_f64(duration),
            playback_time: (playback_time >= 0.0).then(|| Duration::from_secs_f64(playback_time)),
            index: current_idx,
            next,
        })
    }

    #[tracing::instrument(skip(player))]
    #[cfg(feature = "ytdl")]
    pub async fn up_next<I>(player: &PlayerLink, queue_index: I) -> Result<Option<String>, Error>
    where
        I: Into<Option<usize>> + std::fmt::Debug,
    {
        use crate::item::link::VideoLink;

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
        let next = Some(match VideoLink::try_from(next) {
            Ok(l) => {
                tracing::trace!("resolving link");
                l.resolve_link().await
            }
            Err(next) => crate::item::clean_up_path(&next)
                .unwrap_or(&next)
                .to_owned(),
        });
        Ok(next)
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
    pub playing: bool,
    pub volume: f64,
    pub progress: Option<f64>,
    pub playback_time: Option<Duration>,
    pub duration: Duration,
    pub categories: Vec<String>,
    pub index: usize,
    pub next: Option<String>,
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
    current_idx -= start_index; // start index is the new base, so current_idx has to become
                                // relative to that base.
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
