use crate::chosen_index;
use mlib::players::event::{OwnedLibMpvEvent as Ev, OwnedMpvNode};
use std::fmt;
use tokio_stream::StreamExt as _;

pub async fn display() -> anyhow::Result<()> {
    let mut player = std::pin::pin!(chosen_index().subscribe().await?);
    while let Some(ev) = player.next().await {
        match ev {
            Ok(ev) => match ev.event {
                Ev::Shutdown => println!("[{}] shutdown", ev.player_index),
                Ev::LogMessage {
                    prefix,
                    level,
                    text,
                    log_level,
                } => println!(
                    "[{}] log: {prefix}\t{level}\t{text}\t{log_level}",
                    ev.player_index
                ),
                Ev::GetPropertyReply {
                    name,
                    result,
                    reply_userdata,
                } => println!(
                    "[{}] get-property-reply: {name}\t{}\t{reply_userdata}",
                    ev.player_index,
                    DisplayOwnedMpvNode(&result),
                ),
                Ev::SetPropertyReply(d) => {
                    println!("[{}] set-property-reply: {d}", ev.player_index)
                }
                Ev::CommandReply(d) => println!("[{}] command-reply: {d}", ev.player_index),
                Ev::StartFile => println!("[{}] start-file", ev.player_index),
                Ev::EndFile(d) => println!("[{}] end-file: {d}", ev.player_index),
                Ev::FileLoaded => println!("[{}] file-loaded", ev.player_index),
                Ev::ClientMessage(items) => {
                    println!("[{}] client-message: {:?}", ev.player_index, items)
                }
                Ev::VideoReconfig => println!("[{}] video-reconfig", ev.player_index),
                Ev::AudioReconfig => println!("[{}] audio-reconfig", ev.player_index),
                Ev::Seek => println!("[{}] seek", ev.player_index),
                Ev::PlaybackRestart => println!("[{}] playback-restart", ev.player_index),
                Ev::PropertyChange {
                    name,
                    change,
                    reply_userdata,
                } => println!(
                    "[{}] property-change: {name}\t{}\t{reply_userdata}",
                    ev.player_index,
                    DisplayOwnedMpvNode(&change)
                ),
                Ev::QueueOverflow => println!("[{}] queue-overflow", ev.player_index),
                Ev::Deprecated { event_id } => {
                    println!("[{}] deprecated: {event_id}", ev.player_index)
                }
                Ev::Errored(err) => println!("[{}] error: {:?}", ev.player_index, err),
            },
            Err(error) => {
                crate::error!("Event failed"; content: "{error:?}");
            }
        }
    }
    Ok(())
}

struct DisplayOwnedMpvNode<'s>(&'s OwnedMpvNode);

impl fmt::Display for DisplayOwnedMpvNode<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.0 {
            OwnedMpvNode::String(s) => write!(f, "{s}"),
            OwnedMpvNode::OsdString(s) => write!(f, "{s}"),
            OwnedMpvNode::Flag(l) => write!(f, "{l}"),
            OwnedMpvNode::Int64(i) => write!(f, "{i}"),
            OwnedMpvNode::Double(d) => write!(f, "{d}"),
            OwnedMpvNode::Array(owned_mpv_nodes) => {
                write!(f, "[")?;
                for (i, n) in owned_mpv_nodes.iter().enumerate() {
                    write!(f, "{}", DisplayOwnedMpvNode(n))?;
                    if i < owned_mpv_nodes.len().saturating_sub(1) {
                        write!(f, ", ")?;
                    }
                }
                write!(f, "]")
            }
            OwnedMpvNode::Map(hash_map) => {
                write!(f, "{{")?;
                for (i, (k, v)) in hash_map.iter().enumerate() {
                    write!(f, "{k} = {}", DisplayOwnedMpvNode(v))?;
                    if i < hash_map.len().saturating_sub(1) {
                        write!(f, ", ")?;
                    }
                }
                write!(f, "}}")
            }
            OwnedMpvNode::None => write!(f, "<None>"),
            OwnedMpvNode::Invalid(mpv_error) => write!(f, "error({mpv_error})"),
        }
    }
}
