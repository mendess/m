use crate::players::{daemon::Player, event::OwnedLibMpvEvent};
use std::sync::Weak;

#[tracing::instrument("queue wraparound reseter")]
pub async fn reset(player: Weak<Player>) {
    let Some(mut events) = player.upgrade().map(|p| p.subscribe()) else {
        return;
    };
    tracing::info!("starting");
    let mut last_pos = 0;
    while let Ok(e) = events.recv().await {
        let OwnedLibMpvEvent::PropertyChange { name, change, .. } = e.event else {
            continue;
        };
        if name != "playlist-pos" {
            continue;
        }
        let Ok(pos) = change.into_int() else {
            continue;
        };
        if pos < last_pos {
            let Some(player) = player.upgrade() else {
                return;
            };
            player.clear_last_queue();
        }
        last_pos = pos;
    }
    tracing::info!("terminating");
}
