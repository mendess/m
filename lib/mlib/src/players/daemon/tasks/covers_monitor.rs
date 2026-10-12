use crate::players::{
    daemon::Player,
    event::{OwnedLibMpvEvent, OwnedMpvNode},
};
use std::{ffi::OsStr, path::Path, sync::Weak};

#[tracing::instrument("remove album covers from queue")]
pub async fn remove_covers(player: Weak<Player>) {
    let Some(mut events) = player.upgrade().map(|p| p.subscribe()) else {
        return;
    };
    tracing::info!("starting");
    let is_cover = |name: &OwnedMpvNode| {
        let Ok(name) = name.as_str() else {
            return false;
        };
        let path = Path::new(&name);
        path.file_stem() == Some(OsStr::new("cover"))
            && (["jpg", "png"]
                .map(OsStr::new)
                .contains(&path.extension().unwrap_or_default()))
    };
    while let Ok(e) = events.recv().await {
        if let OwnedLibMpvEvent::PropertyChange { name, change, .. } = e.event
            && name == "filename"
            && is_cover(&change)
        {
            let Some(player) = player.upgrade() else {
                return;
            };
            if let Err(error) = player.playlist_remove_current() {
                tracing::error!(?error, ?change, "failed to remove cover")
            }
        }
    }
    tracing::info!("terminating");
}
