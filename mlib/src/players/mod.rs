mod connection;
#[cfg(feature = "player")]
mod daemon;
pub mod error;
pub mod event;
mod legacy_back_compat;
#[cfg(feature = "player")]
mod libmpv_parsing;

use std::{fmt, io, ops::Deref, path::PathBuf, str::FromStr};

use futures_util::Stream;
use serde::{Deserialize, Serialize};

use crate::Item;

#[cfg(feature = "player")]
pub use daemon::start_daemon_if_running_as_daemon;
pub use error::Error;
pub use legacy_back_compat::{legacy_socket_for, override_legacy_socket_base_dir};

use self::event::PlayerEvent;

/// The index of a player
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlayerIndex(Option<usize>);

impl PlayerIndex {
    pub const CURRENT: Self = Self(None);

    pub fn of(index: usize) -> Self {
        Self(Some(index))
    }
}

#[derive(Debug)]
enum StaticOrOwned<T: 'static> {
    Static(&'static T),
    Owned(T),
}

impl<T: 'static> Deref for StaticOrOwned<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        match self {
            StaticOrOwned::Owned(t) => t,
            StaticOrOwned::Static(t) => t,
        }
    }
}

#[derive(Debug)]
pub struct PlayerLink {
    index: PlayerIndex,
    daemon: StaticOrOwned<connection::PlayersDaemonLink>,
}

static CURRENT_LINK: PlayerLink = PlayerLink {
    index: PlayerIndex(None),
    daemon: StaticOrOwned::Static(&connection::PLAYERS),
};

impl PlayerLink {
    pub fn current() -> &'static Self {
        &CURRENT_LINK
    }

    pub fn of(index: usize) -> Self {
        Self {
            index: PlayerIndex(Some(index)),
            daemon: StaticOrOwned::Static(&connection::PLAYERS),
        }
    }

    pub fn linked_to(&self, user: String) -> Self {
        Self {
            index: self.index,
            daemon: StaticOrOwned::Owned(self.daemon.overriding_socket_namespace_with(user)),
        }
    }
}

impl From<PlayerIndex> for PlayerLink {
    fn from(index: PlayerIndex) -> Self {
        Self {
            index,
            daemon: StaticOrOwned::Static(&connection::PLAYERS),
        }
    }
}

impl fmt::Display for PlayerLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.index.0 {
            Some(x) => write!(f, "Player @ {}", x),
            None => write!(f, "Player @ CURRENT"),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    index: PlayerIndex,
    kind: MessageKind,
}

impl Message {
    const fn new(index: PlayerIndex, kind: MessageKind) -> Self {
        Self { index, kind }
    }
    const fn create(items: Vec<Item>, with_video: bool) -> Self {
        Self::new(PlayerIndex(None), MessageKind::Create { items, with_video })
    }
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Direction {
    Next,
    Prev,
}

#[derive(Debug, Serialize, Deserialize)]
enum MessageKind {
    // meta
    Create { items: Vec<Item>, with_video: bool },
    PlayerList,
    LastQueue,
    LastClear,
    LastQueueSet { to: usize },
    Current,
    // actions
    CyclePause,
    Pause,
    Resume,
    QueueClear,
    LoadFile { item: Item },
    LoadList { path: PathBuf },
    QueueMove { from: usize, to: usize },
    QueueRemove { to_remove: usize },
    QueueLoop { start_looping: bool },
    QueueShuffle,
    Quit,
    ChangeVolume { delta: i32 },
    CycleVideo,
    ChangeFile { direction: Direction },
    Seek { seconds: f64 },
    ChangeChapter { direction: Direction, amount: i32 },
    // getters
    ChapterMetadata,
    Filename,
    IsPaused,
    MediaTitle,
    PercentPosition,
    Queue,
    QueueIsLooping,
    QueuePos,
    QueueSize,
    Volume,
    QueueNFilename { at: usize },
    QueueN { at: usize },
    Duration,
    PlaybackTime,
}

#[derive(Debug, Serialize, Deserialize)]
enum Response {
    Create(PlayerIndex),
    Metadata(Metadata),
    MaybeMetadata(Option<Metadata>),
    Bool(bool),
    Text(String),
    Real(f64),
    Item(QueueItem),
    Items(Vec<QueueItem>),
    Integer(i64),
    LoopStatus(LoopStatus),
    PlayerList(Vec<PlayerIndex>),
    MaybeInteger(Option<usize>),
    Unit,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Metadata {
    pub title: String,
    pub index: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueueItem {
    pub filename: String,
    pub status: Option<QueueItemStatus>,
    pub id: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QueueItemStatus {
    pub current: bool,
    pub playing: bool,
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash, Serialize, Deserialize)]
pub enum LoopStatus {
    Inf,
    Force,
    No,
    N(u64),
}

impl FromStr for LoopStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "inf" => Ok(LoopStatus::Inf),
            "force" => Ok(LoopStatus::Force),
            "no" => Ok(LoopStatus::No),
            _ => s.parse().map(LoopStatus::N).map_err(|_| {
                format!("Expected on of 'no', 'force', 'inf' or a number but got {s}")
            }),
        }
    }
}

macro_rules! or_else {
    (($($tt:tt)*) $($rest:tt)*) => ($($tt)*)
}

macro_rules! match_or_else_pat {
    (
    $scrutiny:tt {
        ($($pattern:pat => $res:expr,)*)
        $($rest:tt)*
    }
    ) => (match $scrutiny {
        $($pattern => $res,)*
        x => panic!("invalid response: {x:?}")
    })
}

macro_rules! commands {(
    $(
        $(#[$docs:meta])*
        $name:ident as $kind:ident $({ $($param:ident : $type:ty),+ })?
            $(/ $resp:pat => $res:expr => $r_ty:ty)?
    );* $(;)?
) => {
        impl $crate::players::PlayerLink {
            $(
            $(#[$docs])*
            pub async fn $name(&self, $($($param: $type),*)?)
                -> Result<or_else!($(($r_ty))? (())), $crate::players::Error> {
                let response = self.daemon.exchange(
                    Message::new(
                        self.index,
                        MessageKind::$kind $({ $($param),* })*,
                    )
                ).await??;
                match_or_else_pat!(response {
                    $(($resp => Ok($res),))?
                    (Response::Unit => Ok(()),)
                })
            }
            )*
        }
        $(
        $(#[$docs])*
        pub async fn $name($($($param: $type),*)?)
            -> Result<or_else!($(($r_ty))? (())), $crate::players::Error> {
            let response = PlayerLink::current().daemon.exchange(
                Message::new(
                    PlayerIndex(None),
                    MessageKind::$kind $({ $($param),* })*,
                )
            ).await??;
            match_or_else_pat!(response {
                $(($resp => Ok($res),))?
                (Response::Unit => Ok(()),)
            })
        }
        )*
    };
}

impl PlayerLink {
    pub async fn subscribe(&self) -> Result<impl Stream<Item = io::Result<PlayerEvent>>, Error> {
        Ok(self.daemon.subscribe().await?)
    }
}

pub async fn subscribe() -> Result<impl Stream<Item = io::Result<PlayerEvent>>, Error> {
    Ok(connection::PLAYERS.subscribe().await?)
}

pub async fn wait_for_music_daemon_to_start() {
    connection::PLAYERS.wait_for_daemon_to_spawn().await;
}

/// Create a new player instance, with the given items
pub async fn create(
    items: impl Iterator<Item = &Item>,
    with_video: bool,
) -> Result<PlayerIndex, Error> {
    match connection::PLAYERS
        .exchange(Message::create(items.cloned().collect(), with_video))
        .await??
    {
        Response::Create(index) => Ok(index),
        x => panic!("invalid response: {x:?}"),
    }
}

/// List all running player indexes
pub async fn all() -> Result<Vec<PlayerLink>, Error> {
    match PlayerLink::current()
        .daemon
        .exchange(Message::new(PlayerIndex(None), MessageKind::PlayerList))
        .await??
    {
        Response::PlayerList(l) => Ok(l.into_iter().map(PlayerLink::from).collect()),
        x => panic!("invalid response: {x:?}"),
    }
}

/// Gets the currenly selected player
pub async fn current() -> Result<Option<usize>, Error> {
    match connection::PLAYERS
        .exchange(Message::new(PlayerIndex(None), MessageKind::Current))
        .await??
    {
        Response::MaybeInteger(mi) => Ok(mi),
        x => panic!("invalid response: {x:?}"),
    }
}

pub struct SmartQueueOpts {
    pub no_move: bool,
}

pub struct SmartQueueSummary {
    pub from: usize,
    pub moved_to: usize,
    pub current: usize,
}

impl PlayerLink {
    pub async fn smart_queue(
        &self,
        item: Item,
        opts: SmartQueueOpts,
    ) -> Result<SmartQueueSummary, Error> {
        self.load_file(item.clone()).await?;
        let count = self.queue_size().await?;
        let current = self.queue_pos().await?;
        let queue_summary = if opts.no_move {
            SmartQueueSummary {
                from: count,
                moved_to: count,
                current,
            }
        } else {
            // TODO: this entire logic needs some refactoring
            // there are a lot of edge cases
            // - the queue might have shrunk since the last time we queued

            tracing::debug!("current position: {}", current);
            let mut target = (current + 1) % count;
            tracing::debug!("first target: {}", target);

            if let Some(last) = self.last_queue().await? {
                tracing::debug!("last: {}", last);
                if target <= last {
                    target = (last + 1) % count;
                    tracing::debug!("second target: {}", target);
                }
            };
            let from = count.saturating_sub(1);
            if from != target {
                self.queue_move(from, target).await?;
            }
            self.last_queue_set(target).await?;
            SmartQueueSummary {
                from: count,
                moved_to: target,
                current,
            }
        };
        Ok(queue_summary)
    }
}

pub async fn smart_queue(item: Item, opts: SmartQueueOpts) -> Result<SmartQueueSummary, Error> {
    PlayerLink::current().smart_queue(item, opts).await
}

commands! {
    /// Get the last queued position
    last_queue as LastQueue
        / Response::MaybeInteger(mi) => mi => Option<usize>;
    last_queue_clear as LastClear;
    /// Sets the last queue position.
    last_queue_set as LastQueueSet { to: usize };

    /// Toggle play/pause
    cycle_pause as CyclePause;
    /// Pause the player.
    pause as Pause;
    resume as Resume;
    /// Clears the queue, except for the currently playing song.
    queue_clear as QueueClear;
    /// Adds a file to the queue.
    load_file as LoadFile { item: Item };
    /// Adds all items in a file to the queue.
    load_list as LoadList { path: PathBuf };
    /// Move an item from one postion to the another.
    queue_move as QueueMove { from: usize, to: usize };
    /// Remove an item from the queue.
    queue_remove as QueueRemove { to_remove: usize };
    /// Change whether the queue should loop.
    queue_loop as QueueLoop { start_looping: bool };
    /// Shuffle the queue.
    queue_shuffle as QueueShuffle;
    /// Shuts a player down
    quit as Quit;
    /// Changes the volume of the player
    change_volume as ChangeVolume { delta: i32 };
    /// Toggle video on and off
    toggle_video as CycleVideo;
    /// Change the currently playing file
    change_file as ChangeFile { direction: Direction };
    /// Seek to a new point in the file
    seek as Seek { seconds: f64 };
    /// Jump to a chapter in the file
    change_chapter as ChangeChapter { direction: Direction, amount: i32 };
    /// Get chapter metadata.
    chapter_metadata as ChapterMetadata
        / Response::MaybeMetadata(m) => m => Option<Metadata>;
    /// Get the filename of the currently playing song.
    filename as Filename
        / Response::Text(t) => t => String;
    /// Check if the player is paused.
    is_paused as IsPaused
        / Response::Bool(b) => b => bool;
    /// Get the currently playing media's title, as extracted by ytdl or ffmpeg.
    media_title as MediaTitle
        / Response::Text(t) => t => String;
    /// Get the percent of progress of the curreny song.
    percent_position as PercentPosition
        / Response::Real(r) => r => f64;
    /// Get the current full queue.
    queue as Queue
        / Response::Items(items) => items => Vec<QueueItem>;
    /// Get the queued item at an index
    queue_at as QueueN { at: usize }
        / Response::Item(i) => i => QueueItem;
    /// Check whether the queue is currently looping.
    queue_is_looping as QueueIsLooping
        / Response::LoopStatus(l) => l => LoopStatus;
    /// Get the current queue position.
    queue_pos as QueuePos
        / Response::Integer(i) => i as _ => usize;
    /// Get the queue's size.
    queue_size as QueueSize
        / Response::Integer(i) => i as _ => usize;
    /// Get the player's volume.
    volume as Volume
        / Response::Real(r) => r => f64;
    /// Get the total time of the current track
    duration as Duration
        / Response::Real(r) => r => f64;
    /// Get the total time of the current track
    playback_time as PlaybackTime
        / Response::Real(r) => r => f64;
}
