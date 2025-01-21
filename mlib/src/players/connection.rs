use cli_daemon::Daemon;

use super::{error::MpvResult, event::PlayerEvent, Message, Response};

pub(super) type PlayersDaemonLink = Daemon<Message, MpvResult<Response>, PlayerEvent>;
pub(super) static PLAYERS: PlayersDaemonLink = Daemon::new("m-players");
