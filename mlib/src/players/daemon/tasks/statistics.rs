use crate::{
    item::Item,
    players::{daemon::PlayerEvent, event},
    statistics,
};
use tokio_stream::StreamExt;

#[tracing::instrument(skip_all)]
pub async fn register_statistics_listener(events: impl futures_util::Stream<Item = PlayerEvent>) {
    tracing::info!("starting statistics listener");

    let mut events = std::pin::pin!(events);
    while let Some(event) = events.next().await {
        match event.event {
            event::OwnedLibMpvEvent::PropertyChange {
                name,
                change,
                reply_userdata: _,
            } if name == "filename" => {
                tracing::info!(name, ?change, "property change");
                if let Ok(filename) = change.into_string()
                    && let Err(error) = statistics::played_song(Item::from(filename)).await
                {
                    tracing::error!(?error, "failed to register a played song")
                }
            }
            _ => {}
        }
    }
}
