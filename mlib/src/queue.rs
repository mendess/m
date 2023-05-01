pub use crate::Item;
use crate::{
    item::id_from_path,
    players::{
        error::{Error as PlayerError, MpvError, MpvErrorCode},
        PlayerLink, QueueItem,
    },
    playlist, Error, Link,
};

use futures_util::future::OptionFuture;

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

    pub async fn load_full(index: &PlayerLink) -> Result<Self, Error> {
        Self::load(index, usize::MAX).await
    }

    pub async fn load(index: &PlayerLink, at_most: usize) -> Result<Self, Error> {
        let queue = index.queue().await?;
        let last_queue = index.last_queue().await?;
        let (items, current_idx, playing) = slice_queue(queue, at_most);
        Ok(Self {
            items,
            current_idx,
            playing,
            last_queue,
        })
    }

    pub async fn link(index: &PlayerLink) -> Result<Item, Error> {
        let current_idx = index.queue_pos().await?;
        let current = index.queue_at(current_idx).await?;
        match Item::from(current.filename) {
            Item::Link(l) => Ok(Item::Link(l)),
            Item::File(p) => Ok(id_from_path(&p)
                .map(Link::from_video_id)
                .map(Item::Link)
                .unwrap_or_else(|| Item::File(p))),
            Item::Search(s) => Ok(Item::Search(s)),
        }
    }

    pub async fn current(index: &PlayerLink) -> Result<Current, Error> {
        let media_title = index.media_title().await?;
        let filename = Item::from(index.filename().await?);
        let id = filename.id();
        // TODO: this is wrong
        let title = if media_title.is_empty() {
            filename.to_string()
        } else {
            media_title
        };

        let playing = !index.is_paused().await?;
        let volume = index.volume().await?;
        let progress = match index.percent_position().await {
            Ok(progress) => Some(progress),
            Err(PlayerError::Mpv(MpvError::Raw(MpvErrorCode::PropertyUnavailable))) => None,
            Err(e) => return Err(e.into()),
        };
        let categories = OptionFuture::from(id.map(playlist::find_song))
            .await
            .transpose()?
            .flatten()
            .map(|s| s.categories)
            .unwrap_or_default();

        let chapter = index.chapter_metadata().await.ok().map(|m| m.title);

        let size = index.queue_size().await?;
        let current_idx = index.queue_pos().await?;
        let next = if size == 1 {
            None
        } else {
            Some(index.queue_at((current_idx + 1) % size).await?.filename)
        };
        Ok(Current {
            title,
            chapter,
            playing,
            categories: categories.into_vec(),
            volume,
            progress,
            index: current_idx,
            next,
        })
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

#[derive(Debug, Clone)]
pub struct SongIdent {
    pub index: usize,
    pub item: Item,
}

pub struct Current {
    pub title: String,
    pub chapter: Option<String>,
    pub playing: bool,
    pub volume: f64,
    pub progress: Option<f64>,
    pub categories: Vec<String>,
    pub index: usize,
    pub next: Option<String>,
}

fn slice_queue(mut queue: Vec<QueueItem>, at_most: usize) -> (Vec<SongIdent>, usize, bool) {
    let (mut current_idx, st) = queue
        .iter()
        .enumerate()
        .find_map(|(idx, item)| item.status.map(|st| (idx, st)))
        .unwrap();

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
