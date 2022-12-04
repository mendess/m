use std::{collections::HashMap, future::Future, sync::Arc};

use libmpv::{
    events::{self, Event, PropertyData},
    Format, Mpv, MpvNode, MpvNodeValue,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use super::error::MpvResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerEvent {
    pub player_index: usize,
    pub event: OwnedLibMpvEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OwnedLibMpvEvent {
    Shutdown,
    LogMessage {
        prefix: String,
        level: String,
        text: String,
        log_level: u32,
    },
    GetPropertyReply {
        name: String,
        result: OwnedMpvNode,
        reply_userdata: u64,
    },
    SetPropertyReply(u64),
    CommandReply(u64),
    StartFile,
    EndFile(u32),
    FileLoaded,
    ClientMessage(Vec<Box<str>>),
    VideoReconfig,
    AudioReconfig,
    Seek,
    PlaybackRestart,
    PropertyChange {
        name: String,
        change: OwnedMpvNode,
        reply_userdata: u64,
    },
    QueueOverflow,
    Deprecated {
        event_id: u32,
    },
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

impl From<MpvNode> for OwnedMpvNode {
    fn from(n: MpvNode) -> Self {
        Self::from(&n)
    }
}

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

pub(super) struct EventSubscriber(broadcast::Sender<PlayerEvent>);

impl EventSubscriber {
    pub fn subscribe(&self) -> broadcast::Receiver<PlayerEvent> {
        self.0.subscribe()
    }
}

pub(super) fn event_listener<S, Fut>(
    mpv: Arc<Mpv>,
    player_index: usize,
    shutdown: S,
) -> EventSubscriber
where
    S: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send,
{
    let (tx, _) = broadcast::channel(10);
    tokio::task::spawn_blocking({
        let tx = tx.clone();
        move || {
            let task = move || -> MpvResult<()> {
                let mut events = mpv.create_event_context();
                tracing::debug!(?player_index, "setting up event listener");
                events.enable_all_events()?;
                events.disable_deprecated_events()?;
                events.observe_property("playlist-pos", Format::Int64, 0)?;
                events.observe_property("volume", Format::Double, 0)?;
                events.observe_property("media-title", Format::String, 0)?;
                events.observe_property("pause", Format::Flag, 0)?;
                events.observe_property("chapter", Format::Int64, 0)?;
                events.observe_property("chapter-metadata", Format::Node, 0)?;
                events.enable_event(events::mpv_event_id::Shutdown)?;
                loop {
                    let Some(ev) = events.wait_event(-1.0) else {
                        tracing::trace!("got none event");
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
                            name,
                            change: PropertyData::Int64(-1),
                            reply_userdata: _,
                        } => {
                            tracing::debug!("{name} => -1");
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
                }
                tokio::spawn(async move { shutdown().await });
                let _ = tx.send(PlayerEvent {
                    player_index,
                    event: Event::Shutdown.into(),
                });
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
