use cli_daemon::Daemon;

use super::{error::MpvResult, event::PlayerEvent, Message, Response};

pub(super) type PlayersDaemonLink = Daemon<Message, MpvResult<Response>, PlayerEvent>;
pub(super) static PLAYERS: PlayersDaemonLink = Daemon::new("m-players");

#[tracing::instrument(skip_all)]
#[cfg(feature = "statistics")]
pub async fn register_statistics_listener(
    events: impl futures_util::Stream<Item = PlayerEvent>,
) -> Result<(), super::Error> {
    use tokio_stream::StreamExt;

    tracing::info!("starting statistics listener");

    let mut events = std::pin::pin!(events);
    while let Some(event) = events.next().await {
        match event.event {
            super::event::OwnedLibMpvEvent::PropertyChange {
                name,
                change,
                reply_userdata: _,
            } if name == "filename" => {
                tracing::info!(name, ?change, "property change");
                if let Ok(filename) = change.into_string() {
                    crate::statistics::played_song(crate::item::Item::from(filename)).await?
                }
            }
            _ => {}
        }
    }

    Ok(())
}
