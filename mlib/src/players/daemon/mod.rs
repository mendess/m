mod tasks;

use std::{
    any::type_name,
    num::TryFromIntError,
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::{Duration, SystemTime},
};

use futures_util::{join, stream, Stream, StreamExt};
use libmpv::{FileState, GetData, Mpv, MpvNode};
use regex::Regex;
use tokio::sync::{broadcast, watch, Mutex};

use crate::players::event::event_listener;
use crate::{
    players::{error::MpvError, legacy_socket_for, MessageKind},
    Item,
};

use super::libmpv_parsing;
use super::{
    error::{MpvErrorCode, MpvResult},
    event::{self, PlayerEvent},
    Direction, LoopStatus, Message, Metadata, PlayerIndex, QueueItem, Response,
};

// make fields mod private
use players::Players;
mod players {
    use super::*;
    #[derive(Default)]
    pub struct Players {
        players: Vec<Option<Arc<Player>>>,
    }

    impl Players {
        pub fn add(&mut self, player: Arc<Player>) -> usize {
            for (i, slot) in self.players.iter_mut().enumerate() {
                if slot.is_none() {
                    *slot = Some(player);
                    return i;
                }
            }
            self.players.push(Some(player));
            self.players.len() - 1
        }

        pub fn quit(&mut self, index: usize) -> Option<Arc<Player>> {
            self.players.get_mut(index).and_then(Option::take)
        }

        pub fn get(&self, index: usize) -> Option<&Player> {
            self.players.get(index).and_then(|p| p.as_deref())
        }

        pub fn iter(&self) -> impl Iterator<Item = Option<&Player>> {
            self.players.iter().map(|p| p.as_deref())
        }

        pub fn len(&self) -> usize {
            self.players.len()
        }
    }
}

pub(super) struct PlayersDaemon {
    current_default: watch::Sender<Option<usize>>,
    players: Players,
}

type SharedPlayersDaemon = Arc<Mutex<PlayersDaemon>>;

impl PlayersDaemon {
    fn subscribe_to_current(&self) -> Option<broadcast::Receiver<PlayerEvent>> {
        self.current_default
            .borrow()
            .and_then(|i| self.players.get(i))
            .map(|p| p.subscribe())
    }
}

impl Default for PlayersDaemon {
    fn default() -> Self {
        let (current_default, _) = watch::channel(None);
        Self {
            current_default,
            players: Default::default(),
        }
    }
}

use player::{MpvExt, Player};
mod player {
    use super::*;
    use libmpv::MpvNodeArrayIter;
    use std::ops::Deref;
    use tasks::preemptive_dl::PreemptiveDownload;

    pub struct Player {
        handle: Arc<Mpv>,
        events: event::EventSubscriber,
        last_queue: parking_lot::Mutex<Option<(usize, SystemTime)>>,
        pre_cacher: OnceLock<tasks::preemptive_dl::PreemptiveDownload>,
    }

    impl Player {
        pub fn new(handle: Arc<Mpv>, events: event::EventSubscriber) -> Self {
            Self {
                handle,
                events,
                last_queue: parking_lot::Mutex::new(None),
                pre_cacher: OnceLock::new(),
            }
        }

        pub fn get_last_queue(&self) -> Option<usize> {
            const THREE_HOURS: Duration = Duration::from_secs(60 * 60 * 3);

            let mut last_queue = self.last_queue.lock();
            match &*last_queue {
                Some((idx, set)) => {
                    if set.elapsed().unwrap_or_default() > THREE_HOURS {
                        *last_queue = None;
                        None
                    } else {
                        Some(*idx)
                    }
                }
                None => None,
            }
        }

        pub fn set_last_queue(&self, index: usize) {
            *self.last_queue.lock() = Some((index, SystemTime::now()));
        }

        pub fn clear_last_queue(&self) {
            *self.last_queue.lock() = None;
        }

        pub fn handle(&self) -> &Mpv {
            &self.handle
        }

        pub fn subscribe(&self) -> broadcast::Receiver<PlayerEvent> {
            self.events.subscribe()
        }

        pub fn preemptive_download(&self) -> &PreemptiveDownload {
            self.pre_cacher
                .get_or_init(|| PreemptiveDownload::new(Arc::downgrade(&self.handle)))
        }
    }

    impl Deref for Player {
        type Target = Mpv;
        fn deref(&self) -> &Self::Target {
            self.handle()
        }
    }

    pub trait MpvExt {
        fn simple_prop<T: GetData>(&self, prop: &str) -> MpvResult<T>;
        fn playlist(&self) -> MpvResult<PlaylistIntoIter>;
        fn playlist_move_fixed(&self, from: usize, to: usize) -> MpvResult<()>;
    }

    impl MpvExt for Mpv {
        fn simple_prop<T: GetData>(&self, prop: &str) -> MpvResult<T> {
            self.get_property::<T>(prop).map_err(|e| {
                if let libmpv::Error::Raw(code) = e {
                    use MpvErrorCode::*;
                    const CODES: [MpvErrorCode; 4] = [
                        PropertyNotFound,
                        PropertyFormat,
                        PropertyUnavailable,
                        PropertyError,
                    ];
                    if CODES.iter().any(|c| *c as i32 == code) {
                        tracing::error!("failed to get property {prop}: {code:x}");
                    }
                }
                e.into()
            })
        }

        fn playlist(&self) -> MpvResult<PlaylistIntoIter> {
            let node = self.simple_prop::<MpvNode>("playlist")?;
            node.to_array().ok_or_else(|| MpvError::InvalidData {
                expected: type_name::<Vec<QueueItem>>().to_string(),
                got: format!("{node:?}"),
                error: "wrong node type".into(),
            })?;
            Ok(PlaylistIntoIter { node })
        }

        fn playlist_move_fixed(&self, from: usize, to: usize) -> MpvResult<()> {
            let indices = format!("{from} {to}");
            let (from, to) = indices.split_once(' ').unwrap();
            Ok(self.command("playlist-move", &[from, to])?)
        }
    }

    pub struct PlaylistIntoIter {
        node: MpvNode,
    }

    pub struct PlaylistIter<'s> {
        array: MpvNodeArrayIter<'s>,
    }

    impl<'s> IntoIterator for &'s PlaylistIntoIter {
        type Item = MpvResult<QueueItem>;
        type IntoIter = PlaylistIter<'s>;

        fn into_iter(self) -> Self::IntoIter {
            Self::IntoIter {
                array: self.node.to_array().unwrap(),
            }
        }
    }

    impl Iterator for PlaylistIter<'_> {
        type Item = MpvResult<QueueItem>;
        fn next(&mut self) -> Option<Self::Item> {
            self.array.next().map(libmpv_parsing::parse_queue_item)
        }
    }
}

impl PlayersDaemon {
    pub(crate) async fn create(
        this: Arc<Mutex<Self>>,
        items: Vec<Item>,
        with_video: bool,
    ) -> MpvResult<PlayerIndex> {
        let this_ref = this.clone();
        let mut this_ref = this_ref.lock().await;
        let index = this_ref
            .players
            .iter()
            .position(|slot| slot.is_none())
            .unwrap_or(this_ref.players.len());
        let prepared_items = items
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
        let mpv = Arc::new(Mpv::with_initializer(|mpv| {
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
            mpv.set_property("osc", true)?;

            Ok(())
        })?);

        let events = event_listener(Arc::downgrade(&mpv), index, {
            let this = this.clone();
            async move {
                if let Err(e) = this.lock().await.quit(PlayerIndex::of(index)).await {
                    match e {
                        MpvError::NoMpvInstance => {}
                        e => tracing::error!(?index, ?e, "failed to quit from player"),
                    }
                }
            }
        });

        let player = Arc::new(Player::new(mpv, events));

        tokio::spawn(tasks::last_queue_monitor::reset(Arc::downgrade(&player)));

        player.handle().playlist_load_files(&prepared_items)?;

        for i in items {
            player.preemptive_download().song_queued(&i);
        }

        let index = this_ref.players.add(player);
        tracing::debug!("setting current default to {index}");
        this_ref.current_default.send_replace(Some(index));
        Ok(PlayerIndex::of(index))
    }

    #[cfg(feature = "mpris")]
    pub(super) fn current_default(&self) -> Option<PlayerIndex> {
        self.current_default.borrow().map(PlayerIndex::of)
    }

    pub(super) fn list(&self) -> Vec<PlayerIndex> {
        self.players
            .iter()
            .enumerate()
            .filter_map(|(i, p)| p.is_some().then_some(i))
            .map(PlayerIndex::of)
            .collect()
    }

    #[cfg(feature = "mpris")]
    pub(super) fn len(&self) -> usize {
        self.players.len()
    }

    pub(super) fn last_queue(&mut self, index: PlayerIndex) -> MpvResult<Option<usize>> {
        let pl = index
            .0
            .or_else(|| *self.current_default.borrow())
            .and_then(|index| self.players.get(index))
            .ok_or(MpvError::NoMpvInstance)?;

        Ok(pl.get_last_queue())
    }

    pub(super) fn last_queue_clear(&self, index: PlayerIndex) -> MpvResult<()> {
        let pl = index
            .0
            .or_else(|| *self.current_default.borrow())
            .and_then(|index| self.players.get(index))
            .ok_or(MpvError::NoMpvInstance)?;

        pl.clear_last_queue();

        Ok(())
    }

    pub(super) fn last_queue_set(&self, index: PlayerIndex, to: usize) -> MpvResult<()> {
        let pl = index
            .0
            .or_else(|| *self.current_default.borrow())
            .and_then(|index| self.players.get(index))
            .ok_or(MpvError::NoMpvInstance)?;

        pl.set_last_queue(to);

        Ok(())
    }

    pub(super) fn current_player(&self, index: PlayerIndex) -> MpvResult<&Player> {
        let index = index.0.or_else(|| {
            let index = *self.current_default.borrow();
            tracing::debug!("current player is {index:?}");
            index
        });
        index
            .and_then(|i| self.players.get(i))
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

    pub(super) async fn resume(&self, index: PlayerIndex) -> MpvResult<()> {
        self.current_player(index)?.unpause()?;
        Ok(())
    }

    #[cfg(feature = "mpris")]
    pub(super) async fn jump_to(&self, index: PlayerIndex, pos: usize) -> MpvResult<()> {
        self.current_player(index)?
            .command("playlist-play-index", &[&pos.to_string()])?;
        Ok(())
    }

    pub(super) async fn queue_clear(&self, index: PlayerIndex) -> MpvResult<()> {
        let player = self.current_player(index)?;
        player.playlist_clear()?;
        player.preemptive_download().stop_all();
        Ok(())
    }

    pub(super) async fn load_file(&self, index: PlayerIndex, item: Item) -> MpvResult<()> {
        let player = self.current_player(index)?;
        player.playlist_load_files(&[(
            (&item).try_into().map_err(|_| MpvError::InvalidUtf8)?,
            FileState::AppendPlay,
            None,
        )])?;
        player.preemptive_download().song_queued(&item);
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
        self.current_player(index)?.playlist_move_fixed(from, to)
    }

    pub(super) async fn queue_remove(&self, index: PlayerIndex, to_remove: usize) -> MpvResult<()> {
        let player = self.current_player(index)?;
        if self.queue_is_looping(player)? != LoopStatus::No {
            let pos = simple_prop_logged::<i64>(player, "playlist-pos")?;
            if to_remove as i64 == pos {
                let len = simple_prop_logged::<i64>(player, "playlist-count")?;
                if pos + 1 == len {
                    player.command("playlist-play-index", &["0"])?;
                }
            }
        }

        let item = player.simple_prop::<String>("path").map(Item::from);

        player.playlist_remove_index(to_remove)?;

        match item {
            Ok(item) => {
                if let Item::Link(crate::Link::Video(video)) = &item {
                    player.preemptive_download().song_dequeued(video);
                }
                if let Err(e) = crate::statistics::dequeued_song(item).await {
                    tracing::error!(error = ?e, "failed to record statistics for skipped songs");
                }
            }
            Err(e) => tracing::error!(error = ?e, "failed to get path property"),
        }
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
            .or_else(|| *self.current_default.borrow())
            .ok_or(MpvError::NoMpvInstance)?;

        let player = self.players.quit(index).ok_or(MpvError::NoMpvInstance)?;

        self.current_default.send_if_modified(|cur| {
            if *cur == Some(index) {
                *cur = self
                    .players
                    .iter()
                    .enumerate()
                    .skip(index)
                    .chain(self.players.iter().enumerate().take(index))
                    .find_map(|(i, p)| p.as_ref().map(|_| i));
                true
            } else {
                false
            }
        });
        player.handle().command("quit", &[])?;

        Ok(())
    }

    pub(super) async fn change_volume(&self, index: PlayerIndex, delta: i32) -> MpvResult<()> {
        self.current_player(index)?
            .add_property("volume", delta as isize)?;
        Ok(())
    }

    pub(super) async fn cycle_video(&self, index: PlayerIndex) -> MpvResult<()> {
        self.current_player(index)?.cycle_property("vid", true)?;
        Ok(())
    }

    pub(super) async fn change_file(
        &self,
        index: PlayerIndex,
        direction: Direction,
    ) -> MpvResult<()> {
        let player = self.current_player(index)?;
        let pos: u64 = match simple_prop_logged::<i64>(player, "playlist-pos")?.try_into() {
            Ok(p) => p,
            Err(_) => return Ok(()),
        };
        let count: u64 = match simple_prop_logged::<i64>(player, "playlist-count")?.try_into() {
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
        #[cfg(feature = "statistics")]
        {
            match self.filename(index).await.map(Item::from) {
                Ok(item) => {
                    if let Err(e) = crate::statistics::skipped_song(item).await {
                        tracing::error!(error = ?e, "failed to record statistics for skipped songs");
                    }
                }
                Err(e) => {
                    tracing::error!(error = ?e, "failed to record statistics for skipped songs")
                }
            }
        }
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
        self.current_player(index)?
            .add_property(
                "chapter",
                match direction {
                    Direction::Next => amount as isize,
                    Direction::Prev => -amount as isize,
                },
            )
            .map_err(MpvError::from)
            .map_err(|e| match e {
                MpvError::Raw(MpvErrorCode::Command) => MpvError::FailedToExecute {
                    reason: "this file doesn't have any chapters".into(),
                },
                e => e,
            })?;
        Ok(())
    }

    pub(super) async fn chapter_metadata(&self, index: PlayerIndex) -> MpvResult<Option<Metadata>> {
        use MpvErrorCode as MEC;
        let t = match self
            .current_player(index)?
            .get_property::<MpvNode>("chapter-metadata")
        {
            Ok(t) => t,
            Err(e) => {
                return match e {
                    libmpv::Error::Raw(code) if code == MEC::PropertyUnavailable as i32 => Ok(None),
                    _ => Err(e.into()),
                }
            }
        };
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
        let index = self.simple_prop::<i64>(index, "chapter")?;
        Ok(Some(Metadata {
            title,
            index: index
                .try_into()
                .map_err(|e: TryFromIntError| MpvError::InvalidData {
                    expected: "usize".into(),
                    got: index.to_string(),
                    error: e.to_string(),
                })?,
        }))
    }

    fn simple_prop<T: GetData>(&self, index: PlayerIndex, prop: &str) -> MpvResult<T> {
        self.current_player(index)?.simple_prop(prop)
    }

    pub(super) async fn filename(&self, index: PlayerIndex) -> MpvResult<String> {
        let mut filename = self.simple_prop::<String>(index, "filename")?;
        static YT_ID: OnceLock<Regex> = OnceLock::new();
        let pat = YT_ID.get_or_init(|| Regex::new(r"^[a-zA-Z\-_0-9]{11}$").unwrap());
        // mpv now returns only the video id instead of the full youtube url
        if pat.is_match(&filename) {
            filename.insert_str(0, "https://youtu.be/");
        }
        Ok(filename)
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
        self.current_player(index)?
            .playlist()?
            .into_iter()
            .collect()
    }

    pub(super) fn queue_is_looping(&self, player: &Mpv) -> MpvResult<LoopStatus> {
        let s = simple_prop_logged::<String>(player, "loop-playlist")?;
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
        libmpv_parsing::parse_queue_item(
            self.simple_prop::<MpvNode>(index, &format!("playlist/{at}"))?,
        )
    }

    pub(super) async fn duration(&self, index: PlayerIndex) -> MpvResult<f64> {
        self.simple_prop(index, "duration")
    }

    pub(super) async fn playback_time(&self, index: PlayerIndex) -> MpvResult<f64> {
        self.simple_prop(index, "playback-time")
    }
}

fn simple_prop_logged<T: GetData>(mpv: &Mpv, prop: &str) -> MpvResult<T> {
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
                    tracing::error!("failed to get property {prop}: {code:x}");
                }
            }
            return Err(e.into());
        }
    })
}

async fn handle_messages(
    Message { index, kind }: Message,
    players: SharedPlayersDaemon,
) -> MpvResult<Response> {
    macro_rules! call {
        ($pl:ident.$method:ident($($param:expr),*$(,)?)) => {
            $pl.lock().await.$method($($param),*).await.map(|_| Response::Unit)
        };
        ($pl:ident.$method:ident($($param:expr),*$(,)?) => $ctor:ident) => {
            $pl.lock().await.$method($($param),*).await.map(|v| Response::$ctor(v))
        };
    }
    match kind {
        MessageKind::Create { items, with_video } => {
            PlayersDaemon::create(players, items, with_video)
                .await
                .map(Response::Create)
        }
        MessageKind::PlayerList => Ok(Response::PlayerList(players.lock().await.list())),
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
        MessageKind::Current => Ok(Response::MaybeInteger(
            *players.lock().await.current_default.borrow(),
        )),
        MessageKind::CyclePause => call!(players.cycle_pause(index)),
        MessageKind::Pause => call!(players.pause(index)),
        MessageKind::Resume => call!(players.resume(index)),
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
            call!(players.chapter_metadata(index) => MaybeMetadata)
        }
        MessageKind::Filename => call!(players.filename(index) => Text),
        MessageKind::IsPaused => call!(players.is_paused(index) => Bool),
        MessageKind::MediaTitle => call!(players.media_title(index) => Text),
        MessageKind::PercentPosition => {
            call!(players.percent_position(index) => Real)
        }
        MessageKind::Queue => call!(players.queue(index) => Items),
        MessageKind::QueueIsLooping => {
            let players = players.lock().await;
            let player = players.current_player(index)?;
            players.queue_is_looping(player).map(Response::LoopStatus)
        }
        MessageKind::QueuePos => {
            call!(players.queue_position(index) => Integer)
        }
        MessageKind::QueueSize => call!(players.queue_size(index) => Integer),
        MessageKind::Volume => call!(players.volume(index) => Real),
        MessageKind::QueueNFilename { at } => {
            call!(players.queue_at_filename(index, at) => Text)
        }
        MessageKind::QueueN { at } => {
            call!(players.queue_at(index, at) => Item)
        }
        MessageKind::Duration => {
            call!(players.duration(index) => Real)
        }
        MessageKind::PlaybackTime => {
            call!(players.playback_time(index) => Real)
        }
    }
    .map_err(From::from)
}

async fn event_stream(daemon: SharedPlayersDaemon) -> impl Stream<Item = PlayerEvent> {
    let (current_default, events) = {
        let daemon = daemon.lock().await;
        (
            daemon.current_default.subscribe(),
            daemon.subscribe_to_current(),
        )
    };
    stream::unfold(
        (current_default, events, daemon),
        move |(mut current_default, mut events, daemon)| async move {
            let player_events = async {
                match &mut events {
                    Some(e) => e.recv().await,
                    None => std::future::pending().await,
                }
            };
            let evs = tokio::select! {
                _ = current_default.changed() => {
                    events = daemon.lock().await.subscribe_to_current();
                    None
                },
                Ok(e) = player_events => {
                    Some(e)
                }
            };

            Some((stream::iter(evs), (current_default, events, daemon)))
        },
    )
    .flatten()
}

#[tracing::instrument(name = "players-daemon")]
pub async fn start_daemon_if_running_as_daemon() -> Result<(), super::Error> {
    if let Some(builder) = super::connection::PLAYERS.build_daemon_process().await {
        let players = Arc::new(Mutex::new(PlayersDaemon::default()));
        let run_with_events = builder.run_with_events(
            {
                let players = players.clone();
                move |message| handle_messages(message, players.clone())
            },
            {
                let players = players.clone();
                move || event_stream(players)
            },
        );

        let background_tasks = tasks::register_global_tasks(players);

        let (run_with_events, _) = join!(run_with_events, background_tasks);
        run_with_events?;
    }
    Ok(())
}
