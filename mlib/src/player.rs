pub mod error;

use core::fmt;
use std::{
    any::type_name,
    convert::TryInto,
    io,
    path::PathBuf,
    str::FromStr,
    sync::Arc,
    time::{Duration, SystemTime},
};

use arc_swap::ArcSwapOption;
use cli_daemon::Daemon;
use libmpv::{
    events::{self, Event, PropertyData},
    FileState, Format, GetData, Mpv, MpvNode,
};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::Item;

pub use error::Error;

use error::{MpvError, MpvResult};

use self::error::MpvErrorCode;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PlayerIndex(Option<usize>);

impl fmt::Display for PlayerIndex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            Some(x) => write!(f, "Player @ {}", x),
            None => write!(f, "Player @ CURRENT"),
        }
    }
}

impl PlayerIndex {
    pub const CURRENT: PlayerIndex = PlayerIndex(None);

    pub fn of(index: usize) -> Self {
        Self(Some(index))
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
        Self::new(
            PlayerIndex::CURRENT,
            MessageKind::Create { items, with_video },
        )
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
}

#[derive(Debug, Serialize, Deserialize)]
enum Response {
    Create(PlayerIndex),
    Metadata(Metadata),
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
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QueueItem {
    pub filename: String,
    pub status: Option<QueueItemStatus>,
    pub id: usize,
}

#[derive(Debug, Serialize, Deserialize)]
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
static PLAYERS: Daemon<Message, MpvResult<Response>> = Daemon::new("m-players");

#[derive(Default)]
struct Players {
    current_default: Option<usize>,
    players: Vec<Option<Player>>,
}

struct Player {
    handle: Arc<Mpv>,
    last_queue: Option<(usize, SystemTime)>,
}

impl From<Arc<Mpv>> for Player {
    fn from(handle: Arc<Mpv>) -> Self {
        Self {
            handle,
            last_queue: None,
        }
    }
}

fn parse_queue_item_status(node: MpvNode) -> MpvResult<QueueItemStatus> {
    let mk_err = |error: &'static str| {
        || MpvError::InvalidData {
            expected: type_name::<QueueItemStatus>().to_string(),
            got: format!("{node:?}"),
            error: error.to_string(),
        }
    };
    node.to_map()
        .ok_or_else(mk_err("wrong node type, expected map"))
        .and_then(|m| {
            let mut current = None;
            let mut playing = None;
            for (k, v) in m {
                match k {
                    "current" => {
                        current = Some(
                            v.to_bool()
                                .ok_or_else(mk_err("wrong node type, expected bool"))?,
                        )
                    }
                    "playing" => {
                        playing = Some(
                            v.to_bool()
                                .ok_or_else(mk_err("wrong node type, expected bool"))?,
                        )
                    }
                    _ => {}
                };
            }
            if let (Some(current), Some(playing)) = (current, playing) {
                Ok(QueueItemStatus { current, playing })
            } else {
                Err(mk_err("missing current or playing from node")())
            }
        })
}

fn parse_queue_item(node: MpvNode) -> MpvResult<QueueItem> {
    let mk_err = |error: &'static str| {
        || MpvError::InvalidData {
            expected: type_name::<QueueItem>().to_string(),
            got: format!("{node:?}"),
            error: error.to_string(),
        }
    };
    node.to_map()
        .ok_or_else(mk_err("wrong node type, expected map"))
        .and_then(|i| {
            let mut filename = None;
            let mut status = None;
            let mut current = None;
            let mut playing = None;
            let mut id = None;
            for (k, v) in i {
                match k {
                    "filename" => {
                        filename = Some(
                            v.to_str()
                                .ok_or_else(mk_err("wrong node type, expected string"))?
                                .to_string(),
                        )
                    }
                    "status" => status = Some(parse_queue_item_status(v)?),
                    "current" => {
                        current = Some(
                            v.to_bool()
                                .ok_or_else(mk_err("wrong node type, expected bool"))?,
                        )
                    }
                    "playing" => {
                        playing = Some(
                            v.to_bool()
                                .ok_or_else(mk_err("wrong node type, expected bool"))?,
                        )
                    }
                    "id" => {
                        id = Some(
                            v.to_i64()
                                .ok_or_else(mk_err("wrong node type, expected i64"))?
                                as usize,
                        )
                    }
                    _ => {}
                };
            }
            status = status.or_else(|| {
                Some(QueueItemStatus {
                    current: current?,
                    playing: playing?,
                })
            });
            if let (Some(filename), status, Some(id)) = (filename, status, id) {
                Ok(QueueItem {
                    filename,
                    status,
                    id,
                })
            } else {
                Err(mk_err("missing fields filename or status or id")())
            }
        })
}

static SOCKET_BASE_DIR_OVERRIDE: ArcSwapOption<PathBuf> = ArcSwapOption::const_empty();

pub async fn legacy_socket_for(index: usize) -> String {
    let socket_name = format!(".mpvsocket{index}");
    match &*SOCKET_BASE_DIR_OVERRIDE.load() {
        Some(base) => base.join(socket_name).display().to_string(),
        None => {
            let (path, e) = namespaced_tmp::async_impl::in_user_tmp(&socket_name).await;
            if let Some(e) = e {
                tracing::error!("failed to create socket dir: {:?}", e);
            }
            path.display().to_string()
        }
    }
}

pub fn override_legacy_socket_base_dir(new_base: PathBuf) {
    SOCKET_BASE_DIR_OVERRIDE.store(Some(Arc::new(new_base)));
}

impl Players {
    pub(crate) async fn create(
        this: Arc<Mutex<Self>>,
        items: Vec<Item>,
        with_video: bool,
    ) -> MpvResult<PlayerIndex> {
        let this_ref = this.clone();
        let mut this_ref = this_ref.lock().await;
        #[allow(clippy::never_loop)]
        let index = 'calc: loop {
            for (i, slot) in this_ref.players.iter_mut().enumerate() {
                if slot.is_none() {
                    break 'calc i;
                }
            }
            break this_ref.players.len();
        };
        let items = items
            .iter()
            .flat_map(|i| match i.try_into() {
                Ok(x) => Some(x),
                Err(e) => {
                    tracing::error!(?e, ?i, "invalid item");
                    None
                }
            })
            .map(|i| (i, FileState::AppendPlay, None))
            .collect::<Vec<_>>();
        let legacy_socket = legacy_socket_for(index).await;
        let mpv = Mpv::with_initializer(|mpv| {
            if let Err(e) = mpv.set_property("video", with_video) {
                tracing::error!(error = ?e, "failed to set video to true");
            }
            #[cfg(debug_assertions)]
            {
                mpv.set_property("msg-level", "all=debug")?;
                mpv.set_property("log-file", format!("{legacy_socket}.log"))?;
            }
            mpv.set_property("geometry", "820x466")?;
            mpv.set_property("input-ipc-server", legacy_socket)?;
            mpv.set_property("loop-playlist", items.len() > 1)?;

            Ok(())
        })?;

        mpv.playlist_load_files(&items)?;

        let mpv = Arc::new(mpv);

        tokio::task::spawn_blocking({
            let mpv = mpv.clone();
            move || {
                let task = move || -> MpvResult<()> {
                    let mut events = mpv.create_event_context();
                    events.disable_all_events()?;
                    events.disable_deprecated_events()?;
                    events.observe_property("playlist-pos", Format::Int64, 0)?;
                    events.enable_event(events::mpv_event_id::Shutdown)?;
                    loop {
                        match events.wait_event(-1.0) {
                            Some(Ok(Event::Shutdown)) => {
                                tracing::info!(?index, "got shutdown event");
                                break;
                            }
                            Some(Ok(Event::PropertyChange {
                                name,
                                change: PropertyData::Int64(-1),
                                reply_userdata: _,
                            })) => {
                                tracing::debug!("{name} => -1");
                                break;
                            }
                            Some(e) => {
                                tracing::debug!(?index, event = ?e, "got event");
                            }
                            None => {}
                        }
                    }
                    tokio::spawn(
                        async move { this.lock().await.quit(PlayerIndex::of(index)).await },
                    );
                    tracing::debug!(?index, "player shutting down");
                    Ok(())
                };
                if let Err(e) = task() {
                    tracing::error!(?index, ?e, "player listener failed");
                }
            }
        });

        for (i, slot) in this_ref.players.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = Some(mpv.into());
                this_ref.current_default = Some(i);
                return Ok(PlayerIndex::of(i));
            }
        }
        this_ref.players.push(Some(mpv.into()));
        let index = this_ref.players.len() - 1;
        this_ref.current_default = Some(index);
        Ok(PlayerIndex::of(index))
    }

    pub(super) fn list(&self) -> Vec<PlayerIndex> {
        self.players
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.is_some().then_some(i))
            .map(PlayerIndex::of)
            .collect()
    }

    pub(super) fn last_queue(&mut self, index: PlayerIndex) -> MpvResult<Option<usize>> {
        const THREE_HOURS: Duration = Duration::from_secs(60 * 60 * 3);

        let mut pl = index
            .0
            .or(self.current_default)
            .and_then(|index| self.players.get_mut(index))
            .and_then(|pl| pl.as_mut())
            .ok_or(MpvError::NoMpvInstance)?;

        let (idx, set) = match pl.last_queue.as_ref() {
            Some(lq) => lq,
            None => return Ok(None),
        };
        let now = SystemTime::now();
        if set.duration_since(now).unwrap_or_default() > THREE_HOURS {
            pl.last_queue = None;
            Ok(None)
        } else {
            Ok(Some(*idx))
        }
    }

    pub(super) fn last_queue_clear(&mut self, index: PlayerIndex) -> MpvResult<()> {
        let pl = index
            .0
            .or(self.current_default)
            .and_then(|index| self.players.get_mut(index))
            .and_then(|pl| pl.as_mut())
            .ok_or(MpvError::NoMpvInstance)?;

        pl.last_queue = None;

        Ok(())
    }

    pub(super) fn last_queue_set(&mut self, index: PlayerIndex, to: usize) -> MpvResult<()> {
        let pl = index
            .0
            .or(self.current_default)
            .and_then(|index| self.players.get_mut(index))
            .and_then(|pl| pl.as_mut())
            .ok_or(MpvError::NoMpvInstance)?;

        pl.last_queue = Some((to, SystemTime::now()));

        Ok(())
    }

    fn current_player(&self, index: PlayerIndex) -> MpvResult<&Mpv> {
        index
            .0
            .or(self.current_default)
            .and_then(|i| self.players.get(i))
            .and_then(|m| m.as_ref())
            .map(|p| &*p.handle)
            .ok_or(MpvError::NoMpvInstance)
    }

    pub(super) async fn cycle_pause(&self, index: PlayerIndex) -> MpvResult<()> {
        self.current_player(index)?.cycle_property("pause", true)?;
        Ok(())
    }

    pub(super) async fn pause(&self, index: PlayerIndex) -> MpvResult<()> {
        self.current_player(index)?.pause()?;
        Ok(())
    }

    pub(super) async fn queue_clear(&self, index: PlayerIndex) -> MpvResult<()> {
        self.current_player(index)?.playlist_clear()?;
        Ok(())
    }

    pub(super) async fn load_file(&self, index: PlayerIndex, item: Item) -> MpvResult<()> {
        self.current_player(index)?.playlist_load_files(&[(
            (&item).try_into().map_err(|_| MpvError::InvalidUtf8)?,
            FileState::AppendPlay,
            None,
        )])?;
        Ok(())
    }

    pub(super) async fn load_list(&self, index: PlayerIndex, path: PathBuf) -> MpvResult<()> {
        self.current_player(index)?
            .playlist_load_list(path.to_str().ok_or(MpvError::InvalidUtf8)?, false)?;
        Ok(())
    }

    pub(super) async fn queue_move(
        &self,
        index: PlayerIndex,
        from: usize,
        to: usize,
    ) -> MpvResult<()> {
        let indices = format!("{from} {to}");
        let (from, to) = indices.split_once(' ').unwrap();
        self.current_player(index)?
            .command("playlist-move", &[from, to])?;
        Ok(())
    }

    pub(super) async fn queue_remove(&self, index: PlayerIndex, to_remove: usize) -> MpvResult<()> {
        self.current_player(index)?
            .playlist_remove_index(to_remove)?;
        Ok(())
    }

    pub(super) async fn queue_loop(
        &self,
        index: PlayerIndex,
        start_looping: bool,
    ) -> MpvResult<()> {
        self.current_player(index)?
            .set_property("loop-playlist", if start_looping { "inf" } else { "no" })?;
        Ok(())
    }

    pub(super) async fn queue_shuffle(&self, index: PlayerIndex) -> MpvResult<()> {
        self.current_player(index)?.playlist_shuffle()?;
        Ok(())
    }

    pub(super) async fn quit(&mut self, index: PlayerIndex) -> MpvResult<()> {
        let index = index
            .0
            .or(self.current_default)
            .ok_or(MpvError::NoMpvInstance)?;

        let player = self
            .players
            .get_mut(index)
            .and_then(Option::take)
            .ok_or(MpvError::NoMpvInstance)?;

        if self.current_default == Some(index) {
            self.current_default = self
                .players
                .iter()
                .enumerate()
                .skip(index)
                .chain(self.players[0..index].iter().enumerate())
                .find_map(|(i, p)| p.is_some().then(|| i));
        }
        player.handle.command("quit", &[])?;

        Ok(())
    }

    pub(super) async fn change_volume(&mut self, index: PlayerIndex, delta: i32) -> MpvResult<()> {
        self.current_player(index)?
            .add_property("volume", delta as isize)?;
        Ok(())
    }

    pub(super) async fn cycle_video(&mut self, index: PlayerIndex) -> MpvResult<()> {
        self.current_player(index)?.cycle_property("vid", true)?;
        Ok(())
    }

    pub(super) async fn change_file(
        &self,
        index: PlayerIndex,
        direction: Direction,
    ) -> MpvResult<()> {
        let player = self.current_player(index)?;
        let pos: u64 = match log_property_errors::<i64>(player, "playlist-pos")?.try_into() {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };
        let count: u64 = match log_property_errors::<i64>(player, "playlist-count")?.try_into() {
            Ok(p) if p > 1 => p,
            _ => return Ok(()),
        };
        let new_pos = match direction {
            Direction::Next => (pos + 1) % count,
            Direction::Prev => pos
                .checked_sub(1)
                .unwrap_or_else(|| count.saturating_sub(1)),
        };
        player.command("playlist-play-index", &[&new_pos.to_string()])?;
        Ok(())
    }

    pub(super) async fn seek(&self, index: PlayerIndex, seconds: f64) -> MpvResult<()> {
        self.current_player(index)?.seek_forward(seconds)?;
        Ok(())
    }

    pub(super) async fn change_chapter(
        &self,
        index: PlayerIndex,
        direction: Direction,
        amount: i32,
    ) -> MpvResult<()> {
        self.current_player(index)?.add_property(
            "chapter",
            match direction {
                Direction::Next => amount as isize,
                Direction::Prev => -amount as isize,
            },
        )?;
        Ok(())
    }

    pub(super) async fn chapter_metadata(&self, index: PlayerIndex) -> MpvResult<Metadata> {
        let t = self.simple_prop::<MpvNode>(index, "chapter-metadata")?;
        let title = t
            .to_map()
            .ok_or_else(|| MpvError::InvalidData {
                expected: std::any::type_name::<Metadata>().to_string(),
                got: format!("{t:?}"),
                error: "wrong node type".into(),
            })?
            .find(|(k, _)| *k == "title")
            .ok_or_else(|| MpvError::InvalidData {
                expected: std::any::type_name::<Metadata>().to_string(),
                got: format!("{t:?}"),
                error: "missing field title".into(),
            })?
            .1
            .to_str()
            .map(String::from)
            .ok_or_else(|| MpvError::InvalidData {
                expected: std::any::type_name::<Metadata>().to_string(),
                got: format!("{t:?}"),
                error: "wrong node type, expected string".into(),
            })?;
        Ok(Metadata { title })
    }

    fn simple_prop<T: GetData>(&self, index: PlayerIndex, prop: &str) -> MpvResult<T> {
        log_property_errors(self.current_player(index)?, prop)
    }

    pub(super) async fn filename(&self, index: PlayerIndex) -> MpvResult<String> {
        self.simple_prop(index, "filename")
    }

    pub(super) async fn is_paused(&self, index: PlayerIndex) -> MpvResult<bool> {
        self.simple_prop(index, "pause")
    }

    pub(super) async fn media_title(&self, index: PlayerIndex) -> MpvResult<String> {
        self.simple_prop(index, "media-title")
    }

    pub(super) async fn percent_position(&self, index: PlayerIndex) -> MpvResult<f64> {
        self.simple_prop(index, "percent-pos")
    }

    pub(super) async fn queue(&self, index: PlayerIndex) -> MpvResult<Vec<QueueItem>> {
        let node = self.simple_prop::<MpvNode>(index, "playlist")?;
        node.to_array()
            .ok_or_else(|| MpvError::InvalidData {
                expected: type_name::<Vec<QueueItem>>().to_string(),
                got: format!("{node:?}"),
                error: "wrong node type".into(),
            })?
            .map(parse_queue_item)
            .collect::<Result<Vec<_>, _>>()
    }

    pub(super) async fn queue_is_looping(&self, index: PlayerIndex) -> MpvResult<LoopStatus> {
        let s = self.simple_prop::<String>(index, "loop-playlist")?;
        s.parse::<LoopStatus>()
            .map_err(|error| MpvError::InvalidData {
                expected: type_name::<LoopStatus>().to_string(),
                got: s,
                error,
            })
    }

    pub(super) async fn queue_position(&self, index: PlayerIndex) -> MpvResult<i64> {
        self.simple_prop(index, "playlist-pos")
    }

    pub(super) async fn queue_size(&self, index: PlayerIndex) -> MpvResult<i64> {
        self.simple_prop(index, "playlist-count")
    }

    pub(super) async fn volume(&self, index: PlayerIndex) -> MpvResult<f64> {
        self.simple_prop(index, "volume")
    }

    pub(super) async fn queue_at_filename(
        &self,
        index: PlayerIndex,
        at: usize,
    ) -> MpvResult<String> {
        self.simple_prop(index, &format!("playlist/{at}/filename"))
    }

    pub(super) async fn queue_at(&self, index: PlayerIndex, at: usize) -> MpvResult<QueueItem> {
        parse_queue_item(self.simple_prop::<MpvNode>(index, &format!("playlist/{at}"))?)
    }
}

fn log_property_errors<T: GetData>(mpv: &Mpv, prop: &str) -> MpvResult<T> {
    Ok(match mpv.get_property::<T>(prop) {
        Ok(p) => p,
        Err(e) => {
            if let libmpv::Error::Raw(code) = e {
                use MpvErrorCode::*;
                const CODES: [MpvErrorCode; 4] = [
                    PropertyNotFound,
                    PropertyFormat,
                    PropertyUnavailable,
                    PropertyError,
                ];
                if CODES.iter().any(|c| *c as i32 == code) {
                    tracing::error!("failed to get property {prop}");
                }
            }
            return Err(e.into());
        }
    })
}

pub async fn start_daemon_if_running_as_daemon() -> io::Result<()> {
    if let Some(builder) = PLAYERS.build_daemon_process().await {
        let players = Arc::new(Mutex::new(Players::default()));
        builder
            .run(move |Message { index, kind }| {
                macro_rules! call {
                    ($pl:ident.$method:ident($($param:ident),*$(,)?)) => {
                        $pl.lock().await.$method($($param),*).await.map(|_| Response::Unit)
                    };
                    ($pl:ident.$method:ident($($param:ident),*$(,)?) => $ctor:ident) => {
                        $pl.lock().await.$method($($param),*).await.map(|v| Response::$ctor(v))
                    };
                }
                let players = players.clone();
                async move {
                    match kind {
                        MessageKind::Create { items, with_video } => {
                            Players::create(players, items, with_video)
                                .await
                                .map(Response::Create)
                        }
                        MessageKind::PlayerList => {
                            Ok(Response::PlayerList(players.lock().await.list()))
                        }
                        MessageKind::LastQueue => players
                            .lock()
                            .await
                            .last_queue(index)
                            .map(Response::MaybeInteger),
                        MessageKind::LastClear => players
                            .lock()
                            .await
                            .last_queue_clear(index)
                            .map(|_| Response::Unit),
                        MessageKind::LastQueueSet { to } => players
                            .lock()
                            .await
                            .last_queue_set(index, to)
                            .map(|_| Response::Unit),
                        MessageKind::Current => {
                            Ok(Response::MaybeInteger(players.lock().await.current_default))
                        }
                        MessageKind::CyclePause => call!(players.cycle_pause(index)),
                        MessageKind::Pause => call!(players.pause(index)),
                        MessageKind::QueueClear => call!(players.queue_clear(index)),
                        MessageKind::LoadFile { item } => call!(players.load_file(index, item)),
                        MessageKind::LoadList { path } => call!(players.load_list(index, path)),
                        MessageKind::QueueMove { from, to } => {
                            call!(players.queue_move(index, from, to))
                        }
                        MessageKind::QueueRemove { to_remove } => {
                            call!(players.queue_remove(index, to_remove))
                        }
                        MessageKind::QueueLoop { start_looping } => {
                            call!(players.queue_loop(index, start_looping))
                        }
                        MessageKind::QueueShuffle => call!(players.queue_shuffle(index)),
                        MessageKind::Quit => call!(players.quit(index)),
                        MessageKind::ChangeVolume { delta } => {
                            call!(players.change_volume(index, delta))
                        }
                        MessageKind::CycleVideo => call!(players.cycle_video(index)),
                        MessageKind::ChangeFile { direction } => {
                            call!(players.change_file(index, direction))
                        }
                        MessageKind::Seek { seconds } => call!(players.seek(index, seconds)),
                        MessageKind::ChangeChapter { direction, amount } => {
                            call!(players.change_chapter(index, direction, amount))
                        }
                        MessageKind::ChapterMetadata => {
                            call!(players.chapter_metadata(index) => Metadata)
                        }
                        MessageKind::Filename => call!(players.filename(index) => Text),
                        MessageKind::IsPaused => call!(players.is_paused(index) => Bool),
                        MessageKind::MediaTitle => call!(players.media_title(index) => Text),
                        MessageKind::PercentPosition => {
                            call!(players.percent_position(index) => Real)
                        }
                        MessageKind::Queue => call!(players.queue(index) => Items),
                        MessageKind::QueueIsLooping => {
                            call!(players.queue_is_looping(index) => LoopStatus)
                        }
                        MessageKind::QueuePos => call!(players.queue_position(index) => Integer),
                        MessageKind::QueueSize => call!(players.queue_size(index) => Integer),
                        MessageKind::Volume => call!(players.volume(index) => Real),
                        MessageKind::QueueNFilename { at } => {
                            call!(players.queue_at_filename(index, at) => Text)
                        }
                        MessageKind::QueueN { at } => {
                            call!(players.queue_at(index, at) => Item)
                        }
                    }
                    .map_err(From::from)
                }
            })
            .await?;
    }
    Ok(())
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
        impl $crate::player::PlayerProxy {
            $(
            $(#[$docs])*
            pub async fn $name(self, $($($param: $type),*)?)
                -> Result<or_else!($(($r_ty))? (())), $crate::player::Error> {
                let response = PLAYERS.exchange(
                    Message::new(
                        self.0,
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
        pub async fn $name(index: PlayerIndex, $($($param: $type),*)?)
            -> Result<or_else!($(($r_ty))? (())), $crate::player::Error> {
            let response = PLAYERS.exchange(
                Message::new(
                    index,
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

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Copy)]
pub struct PlayerProxy(PlayerIndex);

pub fn get(index: PlayerIndex) -> PlayerProxy {
    PlayerProxy(index)
}

impl PlayerProxy {
    pub fn as_index(&self) -> PlayerIndex {
        self.0
    }
}

/// Create a new player instance, with the given items
pub async fn create(
    items: impl Iterator<Item = &Item>,
    with_video: bool,
) -> Result<PlayerIndex, Error> {
    match PLAYERS
        .exchange(Message::create(items.cloned().collect(), with_video))
        .await??
    {
        Response::Create(index) => Ok(index),
        x => panic!("invalid response: {x:?}"),
    }
}

/// List all running player indexes
pub async fn all() -> Result<Vec<PlayerIndex>, Error> {
    match PLAYERS
        .exchange(Message::new(PlayerIndex::CURRENT, MessageKind::PlayerList))
        .await??
    {
        Response::PlayerList(l) => Ok(l),
        x => panic!("invalid response: {x:?}"),
    }
}

/// Gets the currenly selected player
pub async fn current() -> Result<Option<usize>, Error> {
    match PLAYERS
        .exchange(Message::new(PlayerIndex::CURRENT, MessageKind::Current))
        .await??
    {
        Response::MaybeInteger(mi) => Ok(mi),
        x => panic!("invalid response: {x:?}"),
    }
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
        / Response::Metadata(m) => m => Metadata;
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
}
