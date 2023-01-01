use crate::{
    item::id_from_path,
    players::{
        error::{Error as PlayerError, MpvError, MpvErrorCode},
        PlayerLink, QueueItemStatus,
    },
    playlist, Error, Link,
};

use std::collections::VecDeque;

use futures_util::future::OptionFuture;

pub struct Queue {
    pub before: VecDeque<SongIdent>,
    pub current: SongIdent,
    pub playing: bool,
    pub after: Vec<SongIdent>,
    pub last_queue: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct SongIdent {
    pub index: usize,
    pub item: Item,
}

pub use crate::Item;

impl Queue {
    pub async fn load(
        index: &PlayerLink,
        before_len: Option<u32>,
        after_len: Option<u32>,
    ) -> Result<Self, Error> {
        let mut play_status = false;
        let mut iter = index.queue().await?.into_iter();

        let mut before = match before_len {
            Some(cap) => VecDeque::with_capacity(cap as usize),
            None => VecDeque::new(),
        };
        let mut next_index = 0;
        let mut next_index = || {
            let i = next_index;
            next_index += 1;
            i
        };
        let mut current = None;
        for i in iter.by_ref() {
            let item = SongIdent {
                index: next_index(),
                item: Item::from(i.filename),
            };
            match i.status {
                Some(QueueItemStatus {
                    current: _,
                    playing,
                }) => {
                    current = Some(item);
                    play_status = playing;
                    break;
                }
                None => {
                    if matches!(before_len, Some(cap) if cap as usize == before.len()) {
                        before.pop_front();
                    }
                    before.push_back(item);
                }
            }
        }
        let after_len = after_len
            .map(|a| {
                a as usize
                    + (before_len
                        .map(|x| x as usize)
                        .unwrap_or(0)
                        .saturating_sub(before.len()))
            })
            .unwrap_or(usize::MAX);
        let after = iter
            .take(after_len)
            .map(|i| SongIdent {
                index: next_index(),
                item: Item::from(i.filename),
            })
            .collect();
        Ok(Self {
            before,
            current: current.unwrap(),
            after,
            last_queue: index.last_queue().await?,
            playing: play_status,
        })
    }

    pub async fn now(index: &PlayerLink, len: u32) -> Result<Self, Error> {
        let before = len / 5;
        let after = len - before - 1;
        Self::load(index, Some(before), Some(after)).await
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
        self.before
            .iter()
            .chain([&self.current])
            .chain(self.after.iter())
    }

    pub fn current_idx(&self) -> usize {
        self.current.index
    }

    pub fn for_each<F: FnMut(&SongIdent), C: FnOnce(&SongIdent)>(&self, mut f: F, c: C) {
        for i in &self.before {
            f(i)
        }
        c(&self.current);
        for i in &self.after {
            f(i)
        }
    }
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
