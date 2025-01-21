use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

use crate::players::daemon;
use futures_util::{Stream, StreamExt, TryFutureExt, TryStreamExt};
use mpris_server::{
    builder::MetadataBuilder, LoopStatus, Metadata, PlaybackRate, PlaybackStatus, PlayerInterface,
    Playlist, PlaylistId, PlaylistOrdering, PlaylistsInterface, RootInterface, Time, TrackId,
    TrackListInterface, Uri, Volume,
};
use tokio::sync::Mutex;
use zbus::fdo;

use crate::{
    players::{event, PlayerIndex},
    Item,
};

use daemon::{event::PlayerEvent, Direction};

pub struct MprisPlayer {
    pub(super) daemon: Arc<Mutex<daemon::PlayersDaemon>>,
}

impl MprisPlayer {
    pub fn new(daemon: Arc<Mutex<daemon::PlayersDaemon>>) -> Self {
        Self { daemon }
    }
}

const C: PlayerIndex = PlayerIndex::CURRENT;

fn to_fdo_err<E: ToString>(e: E) -> fdo::Error {
    fdo::Error::Failed(e.to_string())
}

fn to_zbus_err<E: ToString>(e: E) -> zbus::Error {
    zbus::Error::FDO(Box::new(to_fdo_err(e)))
}

impl RootInterface for MprisPlayer {
    #[tracing::instrument(skip(self))]
    async fn raise(&self) -> fdo::Result<()> {
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn quit(&self) -> fdo::Result<()> {
        self.daemon
            .lock()
            .await
            .quit(C)
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))
    }

    #[tracing::instrument(skip(self))]
    async fn can_quit(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    #[tracing::instrument(skip(self))]
    async fn fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    #[tracing::instrument(skip(self))]
    async fn set_fullscreen(&self, _fullscreen: bool) -> Result<(), zbus::Error> {
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn can_set_fullscreen(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    #[tracing::instrument(skip(self))]
    async fn can_raise(&self) -> fdo::Result<bool> {
        Ok(false)
    }

    #[tracing::instrument(skip(self))]
    async fn has_track_list(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    #[tracing::instrument(skip(self))]
    async fn identity(&self) -> fdo::Result<String> {
        Ok("m".into())
    }

    #[tracing::instrument(skip(self))]
    async fn desktop_entry(&self) -> fdo::Result<String> {
        Ok("".into())
    }

    #[tracing::instrument(skip(self))]
    async fn supported_uri_schemes(&self) -> fdo::Result<Vec<String>> {
        Ok(vec!["file".into(), "http".into()])
    }

    #[tracing::instrument(skip(self))]
    async fn supported_mime_types(&self) -> fdo::Result<Vec<String>> {
        Ok(vec![])
    }
}

impl PlayerInterface for MprisPlayer {
    #[tracing::instrument(skip(self))]
    async fn next(&self) -> fdo::Result<()> {
        self.daemon
            .lock()
            .await
            .change_file(C, Direction::Next)
            .await
            .map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn previous(&self) -> fdo::Result<()> {
        self.daemon
            .lock()
            .await
            .change_file(C, Direction::Prev)
            .await
            .map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn pause(&self) -> fdo::Result<()> {
        self.daemon.lock().await.pause(C).await.map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn play_pause(&self) -> fdo::Result<()> {
        self.daemon
            .lock()
            .await
            .cycle_pause(C)
            .await
            .map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn stop(&self) -> fdo::Result<()> {
        let daemon = self.daemon.lock().await;

        daemon.pause(C).await.map_err(to_fdo_err)?;

        daemon.seek(C, f64::MIN).await.map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn play(&self) -> fdo::Result<()> {
        self.daemon.lock().await.resume(C).await.map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn seek(&self, offset: Time) -> fdo::Result<()> {
        self.daemon
            .lock()
            .await
            .seek(C, offset.as_secs() as f64)
            .await
            .map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn set_position(&self, track_id: TrackId, position: Time) -> fdo::Result<()> {
        let Some(Ok(track_id_pos)) = track_id.as_str().split('/').last().map(str::parse::<i64>)
        else {
            return Err(fdo::Error::InvalidArgs(track_id.to_string()));
        };

        let daemon = self.daemon.lock().await;
        let pos = daemon.queue_position(C).await.map_err(to_fdo_err)?;
        if pos != track_id_pos {
            return Ok(());
        }
        daemon.seek(C, f64::MIN).await.map_err(to_fdo_err)?;
        daemon
            .seek(C, position.as_secs() as f64)
            .await
            .map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn open_uri(&self, uri: String) -> fdo::Result<()> {
        self.daemon
            .lock()
            .await
            .load_file(C, Item::from(uri))
            .await
            .map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn playback_status(&self) -> fdo::Result<PlaybackStatus> {
        let is_paused = self
            .daemon
            .lock()
            .await
            .is_paused(C)
            .await
            .map_err(|e| fdo::Error::Failed(e.to_string()))?;
        if is_paused {
            Ok(PlaybackStatus::Paused)
        } else {
            Ok(PlaybackStatus::Playing)
        }
    }

    #[tracing::instrument(skip(self))]
    async fn loop_status(&self) -> fdo::Result<LoopStatus> {
        let daemon = self.daemon.lock().await;
        let current = daemon.current_player(C).map_err(to_fdo_err)?;
        daemon
            .queue_is_looping(current)
            .map_err(to_fdo_err)
            .map(|status| match status {
                daemon::LoopStatus::Inf | daemon::LoopStatus::Force | daemon::LoopStatus::N(_) => {
                    LoopStatus::Playlist
                }
                daemon::LoopStatus::No => LoopStatus::None,
            })
    }

    #[tracing::instrument(skip(self))]
    async fn set_loop_status(&self, loop_status: LoopStatus) -> zbus::Result<()> {
        self.daemon
            .lock()
            .await
            .queue_loop(
                C,
                matches!(loop_status, LoopStatus::Track | LoopStatus::Playlist),
            )
            .await
            .map_err(to_zbus_err)
    }

    #[tracing::instrument(skip(self))]
    async fn rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    #[tracing::instrument(skip(self))]
    async fn set_rate(&self, _rate: PlaybackRate) -> zbus::Result<()> {
        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn shuffle(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    #[tracing::instrument(skip(self))]
    async fn set_shuffle(&self, shuffle: bool) -> zbus::Result<()> {
        if shuffle {
            self.daemon
                .lock()
                .await
                .queue_shuffle(C)
                .await
                .map_err(to_zbus_err)
        } else {
            Ok(())
        }
    }

    #[tracing::instrument(skip(self))]
    async fn metadata(&self) -> fdo::Result<Metadata> {
        let daemon = self.daemon.lock().await;
        let Some(player) = daemon.current_default() else {
            return Err(fdo::Error::NoServer("no players".into()));
        };
        let pos = daemon.queue_position(C).await.map_err(to_fdo_err)?;
        let id = daemon.queue(C).await.map_err(to_fdo_err)?[pos as usize].id;
        let title = daemon.media_title(C).await.map_err(to_fdo_err)?;
        let chapter_metadata = daemon.chapter_metadata(player).await.map_err(to_fdo_err)?;

        let builder = MetadataBuilder::default().trackid(track_id_on_player(player, id));

        let builder = if let Some(m) = chapter_metadata {
            builder
                .album(title)
                .title(m.title)
                .track_number(m.index as _)
        } else {
            builder.title(title)
        };

        Ok(builder.build())
    }

    #[tracing::instrument(skip(self))]
    async fn volume(&self) -> fdo::Result<Volume> {
        self.daemon.lock().await.volume(C).await.map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn set_volume(&self, volume: Volume) -> zbus::Result<()> {
        let daemon = self.daemon.lock().await;
        daemon
            .change_volume(C, i32::MIN)
            .await
            .map_err(to_zbus_err)?;
        daemon
            .change_volume(C, unsafe { (volume * 100.).to_int_unchecked() })
            .await
            .map_err(to_zbus_err)
    }

    #[tracing::instrument(skip(self))]
    async fn position(&self) -> fdo::Result<Time> {
        self.daemon
            .lock()
            .await
            .playback_time(C)
            .await
            .map_err(to_fdo_err)
            .map(|s| s as i64)
            .map(Time::from_secs)
    }

    #[tracing::instrument(skip(self))]
    async fn minimum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    #[tracing::instrument(skip(self))]
    async fn maximum_rate(&self) -> fdo::Result<PlaybackRate> {
        Ok(1.0)
    }

    #[tracing::instrument(skip(self))]
    async fn can_go_next(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    #[tracing::instrument(skip(self))]
    async fn can_go_previous(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    #[tracing::instrument(skip(self))]
    async fn can_play(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    #[tracing::instrument(skip(self))]
    async fn can_pause(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    #[tracing::instrument(skip(self))]
    async fn can_seek(&self) -> fdo::Result<bool> {
        Ok(true)
    }

    #[tracing::instrument(skip(self))]
    async fn can_control(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

impl TryFrom<PlaylistId> for PlayerIndex {
    type Error = fdo::Error;

    fn try_from(playlist_id: PlaylistId) -> Result<Self, Self::Error> {
        match playlist_id
            .as_str()
            .split('/')
            .last()
            .map(str::parse)
            .map(|r| r.map(PlayerIndex::of))
        {
            Some(Ok(id)) => Ok(id),
            _ => Err(fdo::Error::InvalidArgs(playlist_id.to_string())),
        }
    }
}

const OBJ_PREFIX: &str = "/xyz/mendess/m";
const OBJ_PLAYER: &str = "player";
const OBJ_TRACK_ID: &str = "track";

impl From<PlayerIndex> for PlaylistId {
    fn from(value: PlayerIndex) -> Self {
        PlaylistId::try_from(match value.0 {
            Some(id) => format!("{OBJ_PREFIX}{OBJ_PLAYER}/{id}"),
            None => format!("{OBJ_PREFIX}{OBJ_PLAYER}/current"),
        })
        .unwrap()
    }
}

fn track_id_on_player(pos: PlayerIndex, track_id: usize) -> TrackId {
    TrackId::try_from(match pos.0 {
        Some(id) => format!("{OBJ_PREFIX}/{OBJ_PLAYER}/{id}/{OBJ_TRACK_ID}/{track_id}"),
        None => format!("{OBJ_PREFIX}/{OBJ_PLAYER}/current/{OBJ_TRACK_ID}/{track_id}"),
    })
    .unwrap()
}

fn track_id_to_parts(track_id: &TrackId) -> fdo::Result<(PlayerIndex, usize)> {
    let err = || fdo::Error::InvalidArgs(track_id.to_string());

    let track_id = track_id.strip_prefix(OBJ_PREFIX).ok_or_else(err)?;
    let track_id = track_id.strip_prefix('/').ok_or_else(err)?;

    let mut parts = track_id.split('/');
    if !matches!(parts.next(), Some(s) if s == OBJ_PLAYER) {
        return Err(err());
    };

    let player = match parts.next() {
        Some("current") => PlayerIndex::CURRENT,
        Some(s) => match s.parse().map(PlayerIndex::of) {
            Ok(player) => player,
            Err(_) => return Err(err()),
        },
        None => return Err(err()),
    };

    if !matches!(parts.next(), Some(s) if s == OBJ_TRACK_ID) {
        return Err(err());
    }

    let track = match parts.next().map(str::parse) {
        Some(Ok(id)) => id,
        Some(Err(_)) | None => return Err(err()),
    };

    Ok((player, track))
}

impl From<PlayerIndex> for Playlist {
    fn from(value: PlayerIndex) -> Self {
        Self {
            id: value.into(),
            name: format!("Player {}", value.0.unwrap()),
            icon: Default::default(),
        }
    }
}

impl PlaylistsInterface for MprisPlayer {
    #[tracing::instrument(skip(self))]
    async fn activate_playlist(&self, playlist_id: PlaylistId) -> fdo::Result<()> {
        let id = playlist_id.try_into()?;
        let daemon = self.daemon.lock().await;
        let _exists = daemon.is_paused(id).await.map_err(to_fdo_err)?;
        daemon.pause(C).await.map_err(to_fdo_err)?;
        daemon.resume(id).await.map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn get_playlists(
        &self,
        index: u32,
        max_count: u32,
        _order: PlaylistOrdering,
        _reverse_order: bool,
    ) -> fdo::Result<Vec<Playlist>> {
        let daemon = self.daemon.lock().await;
        let players = daemon.list();
        let slice = players
            .get((index as usize)..)
            .and_then(|slice| slice.get(..(max_count as usize)))
            .unwrap_or_default();

        Ok(slice.iter().map(|i| Playlist::from(*i)).collect())
    }

    #[tracing::instrument(skip(self))]
    async fn playlist_count(&self) -> fdo::Result<u32> {
        Ok(self.daemon.lock().await.len() as u32)
    }

    #[tracing::instrument(skip(self))]
    async fn orderings(&self) -> fdo::Result<Vec<PlaylistOrdering>> {
        Ok(vec![PlaylistOrdering::CreationDate])
    }

    #[tracing::instrument(skip(self))]
    async fn active_playlist(&self) -> fdo::Result<Option<Playlist>> {
        match self.daemon.lock().await.current_default() {
            Some(current) => Ok(Some(Playlist::from(current))),
            None => Ok(None),
        }
    }
}

impl TrackListInterface for MprisPlayer {
    #[tracing::instrument(skip(self))]
    async fn get_tracks_metadata(&self, track_ids: Vec<TrackId>) -> fdo::Result<Vec<Metadata>> {
        let daemon = self.daemon.lock().await;

        let mut queues = HashMap::new();
        let mut metadatas = Vec::new();

        for track_id in track_ids.into_iter() {
            let (player, pos) = track_id_to_parts(&track_id)?;
            let queue = match queues.entry(player) {
                Entry::Vacant(slot) => {
                    let queue = daemon.queue(player).await.map_err(to_fdo_err)?;
                    slot.insert(queue)
                }
                Entry::Occupied(queue) => queue.into_mut(),
            };

            match queue.get(pos) {
                Some(item) => metadatas.push(
                    MetadataBuilder::default()
                        .title(item.filename.clone())
                        .trackid(track_id)
                        .build(),
                ),
                None => metadatas.push(MetadataBuilder::default().build()),
            }
        }

        Ok(metadatas)
    }

    #[tracing::instrument(skip(self))]
    async fn add_track(
        &self,
        uri: Uri,
        after_track: TrackId,
        set_as_current: bool,
    ) -> fdo::Result<()> {
        let (player, pos) = track_id_to_parts(&after_track)?;
        let daemon = self.daemon.lock().await;
        daemon
            .load_file(player, Item::from(uri))
            .await
            .map_err(to_fdo_err)?;

        let len = daemon.queue_size(player).await.map_err(to_fdo_err)?;
        daemon
            .queue_move(player, (len as usize).saturating_sub(1), pos + 1)
            .await
            .map_err(to_fdo_err)?;

        if set_as_current {
            daemon.jump_to(player, pos + 1).await.map_err(to_fdo_err)?;
        }

        Ok(())
    }

    #[tracing::instrument(skip(self))]
    async fn remove_track(&self, track_id: TrackId) -> fdo::Result<()> {
        let (player, pos) = track_id_to_parts(&track_id)?;
        self.daemon
            .lock()
            .await
            .queue_remove(player, pos)
            .await
            .map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn go_to(&self, track_id: TrackId) -> fdo::Result<()> {
        let (player, pos) = track_id_to_parts(&track_id)?;
        self.daemon
            .lock()
            .await
            .jump_to(player, pos)
            .await
            .map_err(to_fdo_err)
    }

    #[tracing::instrument(skip(self))]
    async fn tracks(&self) -> fdo::Result<Vec<TrackId>> {
        let daemon = self.daemon.lock().await;
        let v = futures_util::stream::iter(daemon.list().into_iter())
            .then(|player| daemon.queue(player).map_ok(move |queue| (player, queue)))
            .map_ok(|(player, queue)| {
                queue
                    .into_iter()
                    .map(move |i| track_id_on_player(player, i.id))
            })
            .try_fold(Vec::default(), |mut acc, queue| async {
                acc.extend(queue);
                Ok(acc)
            })
            .await
            .map_err(to_fdo_err)?;

        Ok(v)
    }

    #[tracing::instrument(skip(self))]
    async fn can_edit_tracks(&self) -> fdo::Result<bool> {
        Ok(true)
    }
}

pub async fn signal_mpris_events<S>(server: mpris_server::Server<MprisPlayer>, events: S)
where
    S: Stream<Item = PlayerEvent>,
{
    // Missing Property signals
    // - LoopStatus
    // - Rate (not implemented in general)
    use mpris_server::{Property, Signal};

    // TODO TrackListSignal is not about files currently playing so I need to figure out how to get
    // these fired
    // TrackListProperty as well.
    // use mpris_server::{TrackListSignal, TrackListProperty};

    // PlaylistsSignal does not make sense for this player.
    //
    // Missing
    // - PlaylistsCount when a new player is created.
    use mpris_server::PlaylistsProperty;

    async fn emit_seek(server: &mpris_server::Server<MprisPlayer>, index: PlayerIndex) {
        let playback_time = server.imp().daemon.lock().await.playback_time(index).await;
        match playback_time.map(|t| Time::from_secs(t as i64)) {
            Ok(position) => {
                if let Err(e) = server.emit(Signal::Seeked { position }).await {
                    tracing::error!(?e, "failed signal seeked");
                }
            }
            Err(e) => tracing::error!(?e, "failed to get playback time when signaling"),
        }
    }
    let mut events = std::pin::pin!(events);
    while let Some(event) = events.next().await {
        match event.event {
            event::OwnedLibMpvEvent::Seek => {
                emit_seek(&server, PlayerIndex::of(event.player_index)).await
            }
            event::OwnedLibMpvEvent::Shutdown => {
                let playlist_count = server.imp().daemon.lock().await.len();
                let r = server
                    .playlists_properties_changed([
                        PlaylistsProperty::PlaylistCount(playlist_count as _),
                        PlaylistsProperty::ActivePlaylist(
                            server.imp().active_playlist().await.ok().flatten(),
                        ),
                    ])
                    .await;
                if let Err(e) = r {
                    tracing::error!(
                        ?e,
                        "failed to emit playlists_properties_changed on shutdown"
                    );
                }
            }
            event::OwnedLibMpvEvent::PropertyChange { name, change, .. } => {
                let properties = match name.as_str() {
                    "pause" => {
                        let Ok(paused) = change.into_bool() else {
                            continue;
                        };
                        emit_seek(&server, PlayerIndex::of(event.player_index)).await;
                        Property::PlaybackStatus(if paused {
                            PlaybackStatus::Paused
                        } else {
                            PlaybackStatus::Playing
                        })
                    }
                    "volume" => {
                        let Ok(volume) = change.into_double() else {
                            continue;
                        };
                        Property::Volume(volume)
                    }
                    "media-title" | "chapter-metadata" | "playlist-pos" => {
                        let Ok(meta) = server.imp().metadata().await else {
                            continue;
                        };
                        Property::Metadata(meta)
                    }
                    _ => continue,
                };
                if let Err(e) = server.properties_changed([properties]).await {
                    tracing::error!(?e, "failed to emit properties_changed playback status");
                }
            }
            event::OwnedLibMpvEvent::StartFile
            | event::OwnedLibMpvEvent::EndFile(_)
            | event::OwnedLibMpvEvent::PlaybackRestart
            | event::OwnedLibMpvEvent::FileLoaded => { /* TODO maybe these are important */ }
            event::OwnedLibMpvEvent::GetPropertyReply { .. }
            | event::OwnedLibMpvEvent::SetPropertyReply(_)
            | event::OwnedLibMpvEvent::CommandReply(_)
            | event::OwnedLibMpvEvent::QueueOverflow
            | event::OwnedLibMpvEvent::ClientMessage(_)
            | event::OwnedLibMpvEvent::VideoReconfig
            | event::OwnedLibMpvEvent::AudioReconfig
            | event::OwnedLibMpvEvent::Deprecated { .. }
            | event::OwnedLibMpvEvent::LogMessage { .. }
            | event::OwnedLibMpvEvent::Errored(_) => {}
        }
    }
}
