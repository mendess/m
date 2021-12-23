use crate::{
    id_from_path,
    playlist::Playlist,
    socket::{self, cmds::QueueItemStatus, MpvSocket},
    Error, Link,
};

use std::{
    collections::VecDeque,
    fmt::Display,
    io,
    path::{Path, PathBuf},
};

use futures_util::future::OptionFuture;

pub struct Queue {
    before: VecDeque<Item>,
    current: Item,
    playing: bool,
    after: Vec<Item>,
    last_queue: Option<usize>,
}

#[derive(Debug)]
enum Item {
    Link(Link),
    File(PathBuf),
}

impl Item {
    fn id(&self) -> Option<&str> {
        match self {
            Item::Link(l) => Some(l.id()),
            Item::File(p) => id_from_path(p),
        }
    }
}

impl Display for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Item::Link(l) => write!(f, "{}", l.as_str()),
            Item::File(p) => write!(
                f,
                "{}",
                p.file_stem()
                    .map(|s| s.to_string_lossy())
                    .unwrap_or_default()
            ),
        }
    }
}

impl From<String> for Item {
    fn from(s: String) -> Self {
        if s.starts_with("http") {
            Item::Link(Link::from_url(s))
        } else {
            Item::File(PathBuf::from(s))
        }
    }
}

impl Queue {
    async fn load(
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
        let mut current = None;
        for i in iter.by_ref() {
            let item = Item::from(i.filename);
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
            .map(|i| Item::from(i.filename))
            .collect();
        Ok(Self {
            before,
            current: current.unwrap(),
            after,
            last_queue: fetch_last_queue().await?,
            playing: play_status,
        })
    }

    pub async fn now(socket: &mut MpvSocket, len: usize) -> Result<Self, Error> {
        let before = len / 5;
        let after = len - before - 1;
        Self::load(socket, Some(before), Some(after)).await
    }

    pub async fn link(socket: &mut MpvSocket) -> Result<Option<String>, Error> {
        let current_idx = socket.compute(socket::cmds::QueuePos).await?;
        let current = socket.compute(socket::cmds::QueueN(current_idx)).await?;
        match Item::from(current.filename) {
            Item::Link(l) => Ok(Some(l.into_string())),
            Item::File(p) => Ok(id_from_path(&p).map(|p| format!("https://youtu.be/{}", p))),
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
        let categories = OptionFuture::from(id.map(Playlist::find_song))
            .await
            .transpose()?
            .flatten()
            .map(|s| s.categories)
            .unwrap_or_default();

        // TODO: chapter-metadata

        let current_idx = socket.compute(socket::cmds::QueuePos).await?;
        // TODO: this can fail, we can be at the end, in with case I have to wrap around
        let up_next = socket
            .compute(socket::cmds::QueueNFilename(current_idx))
            .await?;
        Ok(Current {
            title,
            playing,
            categories,
            volume,
            progress,
            next: Some(up_next),
        })
    }
}

pub struct Current {
    pub title: String,
    pub playing: bool,
    pub volume: f64,
    pub progress: f64,
    pub categories: Vec<String>,
    pub next: Option<String>,
}

async fn fetch_last_queue() -> Result<Option<usize>, Error> {
    let mut path = Playlist::path()?;
    let mut name = path
        .file_name()
        .expect("playlist path to have a filename")
        .to_os_string();
    path.pop();
    name.push("_last_queue");
    path.push(name);
    match tokio::fs::read_to_string(&path).await {
        Ok(s) => match s.trim().parse() {
            Ok(n) => Ok(Some(n)),
            Err(_) => {
                tracing::error!("failed to parse last queue, file corrupted? '{:?}'", path);
                Ok(None)
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::id_from_path;

    #[test]
    fn trivial() {
        assert_eq!(
            Some("AAA"),
            id_from_path(&PathBuf::from("Song Name 😎=AAA=m.mkv"))
        )
    }

    #[test]
    fn no_id() {
        assert_eq!(
            None,
            id_from_path(&PathBuf::from("Some-song-title-AAA.mkv"))
        )
    }

    #[test]
    fn no_id_2() {
        assert_eq!(
            None,
            id_from_path(&PathBuf::from("Some-song-title-AAA=m.mkv"))
        )
    }
}
