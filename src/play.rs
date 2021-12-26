use futures_util::stream::{self, StreamExt};
use mlib::{downloaded::check_cache, queue::Item, socket::MpvSocket};
use tokio::process::Command;

pub async fn play(items: Vec<Item>, with_video: bool) -> anyhow::Result<()> {
    let items = stream::iter(items)
        .map(|i| async move {
            match i {
                Item::Link(l) => check_cache(l).await,
                x => x,
            }
        })
        .buffered(16)
        .collect::<Vec<_>>()
        .await;

    let (first, tail) = items.split_at(20.clamp(0, items.len().saturating_sub(1)));

    let mut mpv = Command::new("mpv");
    mpv.arg("--geometry=820x466");
    mpv.arg(format!(
        "--input-ipc-server={}",
        MpvSocket::new().await?.path().display()
    ));
    if !with_video {
        mpv.arg("--no-video");
    }
    if first.len() > 1 {
        mpv.arg("--loop-playlist");
    }
    mpv.args(first);

    if !tail.is_empty() {
        todo!("batch queue file")
    }

    mpv.spawn()?;

    Ok(())
}
