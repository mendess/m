use crate::{
    id_from_path, playlist,
    socket::{self, cmds::QueueItemStatus, MpvSocket},
    Error, Link,
};

use std::collections::{HashSet, VecDeque};

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
        socket: &mut MpvSocket,
        before_len: Option<usize>,
        after_len: Option<usize>,
    ) -> Result<Self, Error> {
        let mut play_status = false;
        let mut iter = socket.compute(socket::cmds::Queue).await?.into_iter();

        let mut before = match before_len {
            Some(cap) => VecDeque::with_capacity(cap),
            None => VecDeque::new(),
        };
        let mut index = 0;
        let mut index = || {
            let i = index;
            index += 1;
            i
        };
        let mut current = None;
        for i in iter.by_ref() {
            let item = SongIdent {
                index: index(),
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
                    if matches!(before_len, Some(cap) if cap == before.len()) {
                        before.pop_front();
                    }
                    before.push_back(item);
                }
            }
        }
        let after_len = after_len
            .map(|a| a + (before_len.unwrap_or(0).saturating_sub(before.len())))
            .unwrap_or(usize::MAX);
        let after = iter
            .take(after_len)
            .map(|i| SongIdent {
                index: index(),
                item: Item::from(i.filename),
            })
            .collect();
        Ok(Self {
            before,
            current: current.unwrap(),
            after,
            last_queue: last::fetch().await?,
            playing: play_status,
        })
    }

    pub async fn now(socket: &mut MpvSocket, len: usize) -> Result<Self, Error> {
        let before = len / 5;
        let after = len - before - 1;
        Self::load(socket, Some(before), Some(after)).await
    }

    pub async fn link(socket: &mut MpvSocket) -> Result<Item, Error> {
        let current_idx = socket.compute(socket::cmds::QueuePos).await?;
        let current = socket.compute(socket::cmds::QueueN(current_idx)).await?;
        match Item::from(current.filename) {
            Item::Link(l) => Ok(Item::Link(l)),
            Item::File(p) => Ok(id_from_path(&p)
                .map(Link::from_id)
                .map(Item::Link)
                .unwrap_or_else(|| Item::File(p))),
            Item::Search(s) => Ok(Item::Search(s)),
        }
    }

    pub async fn current(socket: &mut MpvSocket) -> Result<Current, Error> {
        let media_title = socket.compute(socket::cmds::MediaTitle).await?;
        let filename = Item::from(socket.compute(socket::cmds::Filename).await?);
        let id = filename.id();
        // TODO: this is wrong
        let title = if media_title.is_empty() {
            filename.to_string()
        } else {
            media_title
        };

        let playing = !socket.compute(socket::cmds::IsPaused).await?;
        let volume = socket.compute(socket::cmds::Volume).await?;
        let progress = socket.compute(socket::cmds::PercentPosition).await?;
        let categories = OptionFuture::from(id.map(playlist::find_song))
            .await
            .transpose()?
            .flatten()
            .map(|s| s.categories)
            .unwrap_or_default();

        let chapter = socket.compute(socket::cmds::ChapterMetadata).await.ok().map(|m| m.title);

        let size = socket.compute(socket::cmds::QueueSize).await?;
        let next = if size == 1 {
            None
        } else {
            let current_idx = socket.compute(socket::cmds::QueuePos).await?;
            Some(
                socket
                    .compute(socket::cmds::QueueNFilename((current_idx + 1) % size))
                    .await?,
            )
        };
        Ok(Current {
            title,
            chapter,
            playing,
            categories,
            volume,
            progress,
            next,
        })
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &SongIdent> {
        self.before
            .iter()
            .chain(Some(&self.current))
            .chain(self.after.iter())
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
    pub progress: f64,
    pub categories: HashSet<String>,
    pub next: Option<String>,
}

pub mod last;
