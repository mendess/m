use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[cfg(feature = "player")]
use super::error::MpvResult;
#[cfg(feature = "player")]
use libmpv::{
    events::{self, Event, PropertyData},
    Format, Mpv, MpvNode, MpvNodeValue,
};
#[cfg(feature = "player")]
use std::{future::Future, sync::Weak, thread, time::Duration};
#[cfg(feature = "player")]
use tokio::sync::broadcast;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerEvent {
    pub player_index: usize,
    pub event: OwnedLibMpvEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OwnedLibMpvEvent {
    /// Received when the player is shutting down
    Shutdown,
    /// *Has not been tested*, received when explicitly asked to MPV
    LogMessage {
        prefix: String,
        level: String,
        text: String,
        log_level: u32,
    },
    /// Received when using get_property_async
    GetPropertyReply {
        name: String,
        result: OwnedMpvNode,
        reply_userdata: u64,
    },
    /// Received when using set_property_async
    SetPropertyReply(u64),
    /// Received when using command_async
    CommandReply(u64),
    /// Event received when a new file is playing
    StartFile,
    /// Event received when the file being played currently has stopped, for an error or not
    EndFile(u32),
    /// Event received when a file has been *loaded*, but has not been started
    FileLoaded,
    ClientMessage(Vec<Box<str>>),
    VideoReconfig,
    AudioReconfig,
    /// The player changed current position
    Seek,
    PlaybackRestart,
    /// Received when used with observe_property
    PropertyChange {
        name: String,
        change: OwnedMpvNode,
        reply_userdata: u64,
    },
    /// Received when the Event Queue is full
    QueueOverflow,
    /// A deprecated event
    Deprecated {
        event_id: u32,
    },
    /// Emited when an error occurred while receiving an event.
    Errored(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OwnedMpvNode {
    String(String),
    OsdString(String),
    Flag(bool),
    Int64(i64),
    Double(f64),
    Array(Vec<OwnedMpvNode>),
    Map(HashMap<String, OwnedMpvNode>),
    None,
    Invalid(super::error::MpvError),
}

impl OwnedMpvNode {
    pub fn into_string(self) -> Result<String, super::error::MpvError> {
        match self {
            Self::String(s) => Ok(s),
            Self::OsdString(s) => Ok(s),
            Self::Invalid(e) => Err(e),
            _ => panic!("{self:?} is not a string"),
        }
    }

    pub fn into_bool(self) -> Result<bool, super::error::MpvError> {
        match self {
            Self::Flag(flag) => Ok(flag),
            Self::Invalid(e) => Err(e),
            _ => panic!("{self:?} is not a bool"),
        }
    }

    pub fn into_int(self) -> Result<i64, super::error::MpvError> {
        match self {
            Self::Int64(i) => Ok(i),
            Self::Invalid(e) => Err(e),
            _ => panic!("{self:?} is not an int"),
        }
    }

    pub fn into_double(self) -> Result<f64, super::error::MpvError> {
        match self {
            Self::Double(d) => Ok(d),
            Self::Invalid(e) => Err(e),
            _ => panic!("{self:?} is not a double"),
        }
    }

    pub fn into_array(self) -> Result<Vec<Self>, super::error::MpvError> {
        match self {
            Self::Array(a) => Ok(a),
            Self::Invalid(e) => Err(e),
            _ => panic!("{self:?} is not an array"),
        }
    }

    pub fn into_map(self) -> Result<HashMap<String, Self>, super::error::MpvError> {
        match self {
            Self::Map(m) => Ok(m),
            Self::Invalid(e) => Err(e),
            _ => panic!("{self:?} is not a map"),
        }
    }
}

#[cfg(feature = "player")]
impl From<MpvNode> for OwnedMpvNode {
    fn from(n: MpvNode) -> Self {
        Self::from(&n)
    }
}

#[cfg(feature = "player")]
impl From<&MpvNode> for OwnedMpvNode {
    fn from(n: &MpvNode) -> Self {
        match n.value() {
            Ok(MpvNodeValue::String(s)) => OwnedMpvNode::String(s.into()),
            Ok(MpvNodeValue::Flag(f)) => OwnedMpvNode::Flag(f),
            Ok(MpvNodeValue::Int64(i)) => OwnedMpvNode::Int64(i),
            Ok(MpvNodeValue::Double(d)) => OwnedMpvNode::Double(d),
            Ok(MpvNodeValue::Array(a)) => OwnedMpvNode::Array(a.map(Into::into).collect()),
            Ok(MpvNodeValue::Map(m)) => {
                OwnedMpvNode::Map(m.map(|(k, v)| (k.into(), v.into())).collect())
            }
            Ok(MpvNodeValue::None) => OwnedMpvNode::None,
            Err(e) => OwnedMpvNode::Invalid(e.into()),
        }
    }
}

#[cfg(feature = "player")]
impl From<PropertyData<'_>> for OwnedMpvNode {
    fn from(p: PropertyData<'_>) -> Self {
        match p {
            PropertyData::Str(s) => OwnedMpvNode::String(s.into()),
            PropertyData::OsdStr(s) => OwnedMpvNode::OsdString(s.into()),
            PropertyData::Flag(f) => OwnedMpvNode::Flag(f),
            PropertyData::Int64(i) => OwnedMpvNode::Int64(i),
            PropertyData::Double(d) => OwnedMpvNode::Double(d),
            PropertyData::Node(n) => n.into(),
        }
    }
}

const _: fn() = || {
    fn is_send<T: Send>() {}
    is_send::<OwnedLibMpvEvent>();
};

#[cfg(feature = "player")]
impl From<Event<'_>> for OwnedLibMpvEvent {
    fn from(e: Event<'_>) -> Self {
        match e {
            Event::Shutdown => OwnedLibMpvEvent::Shutdown,
            Event::LogMessage {
                prefix,
                level,
                text,
                log_level,
            } => OwnedLibMpvEvent::LogMessage {
                prefix: prefix.into(),
                level: level.into(),
                text: text.into(),
                log_level,
            },
            Event::GetPropertyReply {
                name,
                result,
                reply_userdata,
            } => OwnedLibMpvEvent::GetPropertyReply {
                name: name.into(),
                result: result.into(),
                reply_userdata,
            },
            Event::SetPropertyReply(r) => OwnedLibMpvEvent::SetPropertyReply(r),
            Event::CommandReply(r) => OwnedLibMpvEvent::CommandReply(r),
            Event::StartFile => OwnedLibMpvEvent::StartFile,
            Event::EndFile(e) => OwnedLibMpvEvent::EndFile(e),
            Event::FileLoaded => OwnedLibMpvEvent::FileLoaded,
            Event::ClientMessage(m) => OwnedLibMpvEvent::ClientMessage(
                m.iter()
                    .map(ToString::to_string)
                    .map(String::into_boxed_str)
                    .collect(),
            ),
            Event::VideoReconfig => OwnedLibMpvEvent::VideoReconfig,
            Event::AudioReconfig => OwnedLibMpvEvent::AudioReconfig,
            Event::Seek => OwnedLibMpvEvent::Seek,
            Event::PlaybackRestart => OwnedLibMpvEvent::PlaybackRestart,
            Event::PropertyChange {
                name,
                change,
                reply_userdata,
            } => OwnedLibMpvEvent::PropertyChange {
                name: name.into(),
                change: change.into(),
                reply_userdata,
            },
            Event::QueueOverflow => OwnedLibMpvEvent::QueueOverflow,
            Event::Deprecated(d) => OwnedLibMpvEvent::Deprecated {
                event_id: d.event_id,
            },
        }
    }
}

#[cfg(feature = "player")]
pub(super) struct EventSubscriber(broadcast::Sender<PlayerEvent>);

#[cfg(feature = "player")]
impl EventSubscriber {
    pub fn subscribe(&self) -> broadcast::Receiver<PlayerEvent> {
        self.0.subscribe()
    }
}

#[cfg(feature = "player")]
pub(super) fn event_listener<S>(mpv: Weak<Mpv>, player_index: usize, shutdown: S) -> EventSubscriber
where
    S: Future<Output = ()> + Send + 'static,
{
    let (tx, _) = broadcast::channel(10);
    tokio::task::spawn_blocking({
        let tx = tx.clone();
        move || {
            let task = move || -> MpvResult<()> {
                thread::sleep(Duration::from_secs_f32(0.5));
                let Some(mpv) = mpv.upgrade() else {
                    return Ok(());
                };
                let mut events = mpv.create_event_context();
                tracing::debug!(?player_index, "setting up event listener");
                events.observe_property("filename", Format::String, 0)?;
                events.observe_property("playlist-pos", Format::Int64, 0)?;
                events.observe_property("volume", Format::Double, 0)?;
                events.observe_property("media-title", Format::String, 0)?;
                events.observe_property("pause", Format::Flag, 0)?;
                events.observe_property("chapter", Format::Int64, 0)?;
                events.observe_property("chapter-metadata", Format::Node, 0)?;
                events.enable_event(events::mpv_event_id::Shutdown)?;
                events.enable_event(events::mpv_event_id::FileLoaded)?;
                events.enable_event(events::mpv_event_id::StartFile)?;
                let mut first_event = true;
                loop {
                    let Some(ev) = events.wait_event(-1. /* never timeout */) else {
                        tracing::debug!("got none event");
                        continue;
                    };
                    let Ok(ev) = ev else {
                        tracing::error!(?ev, "error receiving event");
                        continue;
                    };
                    match &ev {
                        Event::Shutdown => {
                            tracing::info!(?player_index, "got shutdown event");
                            break;
                        }
                        Event::PropertyChange {
                            name: "playlist-pos",
                            change: PropertyData::Int64(-1),
                            reply_userdata: _,
                        } if !first_event => {
                            tracing::debug!("playlist-pos => -1");
                            break;
                        }
                        Event::Deprecated(_) => continue,
                        e => {
                            tracing::debug!(?player_index, event = ?e, "got event");
                        }
                    }
                    let _ = tx.send(PlayerEvent {
                        player_index,
                        event: ev.into(),
                    });
                    first_event = false;
                }
                drop(mpv);
                let _ = tx.send(PlayerEvent {
                    player_index,
                    event: Event::Shutdown.into(),
                });
                tokio::spawn(shutdown);
                tracing::debug!(?player_index, "player shutting down");
                Ok(())
            };
            if let Err(e) = task() {
                tracing::error!(?player_index, ?e, "player listener failed");
            }
        }
    });
    EventSubscriber(tx)
}
