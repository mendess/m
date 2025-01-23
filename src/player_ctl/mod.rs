mod interactive;

pub use interactive::interactive;

use super::arg_parse::Amount;

use anyhow::Context;
use mlib::{players, queue::Queue};

use crate::{chosen_index, notify};

pub async fn resume() -> anyhow::Result<()> {
    Ok(chosen_index().resume().await?)
}

pub async fn pause() -> anyhow::Result<()> {
    Ok(chosen_index().pause().await?)
}

pub async fn quit() -> anyhow::Result<()> {
    Ok(chosen_index().quit().await?)
}

/// cycle pause
pub async fn cycle_pause() -> anyhow::Result<()> {
    Ok(chosen_index().cycle_pause().await?)
}

pub async fn vu<A>(amount: A) -> anyhow::Result<()>
where
    A: Into<Amount>,
{
    let Amount { amount } = amount.into();
    Ok(chosen_index().change_volume(amount.unwrap_or(2)).await?)
}

pub async fn vd<A>(amount: A) -> anyhow::Result<()>
where
    A: Into<Amount>,
{
    let Amount { amount } = amount.into();
    Ok(chosen_index().change_volume(-amount.unwrap_or(2)).await?)
}

pub async fn toggle_video() -> anyhow::Result<()> {
    Ok(chosen_index().toggle_video().await?)
}

pub async fn next_file<A>(amount: A) -> anyhow::Result<()>
where
    A: Into<Amount>,
{
    let Amount { amount } = amount.into();
    let player = chosen_index();
    for _ in 0..amount.unwrap_or(1) {
        tracing::debug!("going to next file");
        player.change_file(players::Direction::Next).await?;
    }
    Ok(())
}

pub async fn prev_file<A>(amount: A) -> anyhow::Result<()>
where
    A: Into<Amount>,
{
    let Amount { amount } = amount.into();
    let player = chosen_index();
    for _ in 0..amount.unwrap_or(1) {
        player.change_file(players::Direction::Prev).await?;
    }
    Ok(())
}

pub async fn frwd<A>(amount: A) -> anyhow::Result<()>
where
    A: Into<Amount>,
{
    let Amount { amount } = amount.into();
    Ok(chosen_index().seek(amount.unwrap_or(10) as f64).await?)
}

pub async fn back<A>(amount: A) -> anyhow::Result<()>
where
    A: Into<Amount>,
{
    let Amount { amount } = amount.into();
    Ok(chosen_index().seek(-(amount.unwrap_or(10) as f64)).await?)
}

pub async fn next<A>(amount: A) -> anyhow::Result<()>
where
    A: Into<Amount>,
{
    let Amount { amount } = amount.into();
    Ok(chosen_index()
        .change_chapter(players::Direction::Next, amount.unwrap_or(1))
        .await?)
}

pub async fn prev<A>(amount: A) -> anyhow::Result<()>
where
    A: Into<Amount>,
{
    let Amount { amount } = amount.into();
    Ok(chosen_index()
        .change_chapter(players::Direction::Prev, amount.unwrap_or(1))
        .await?)
}

pub async fn shuffle() -> anyhow::Result<()> {
    Ok(players::queue_shuffle().await?)
}

pub async fn toggle_loop() -> anyhow::Result<()> {
    let player = chosen_index();
    let looping = match player.queue_is_looping().await? {
        players::LoopStatus::Inf => false,
        players::LoopStatus::No => true,
        _ => false,
    };
    player.queue_loop(looping).await?;
    if looping {
        notify!("now looping");
    } else {
        notify!("not looping");
    }
    Ok(())
}

pub async fn status() -> anyhow::Result<()> {
    let all = players::all().await?;
    for player in all {
        let current = Queue::current(&player, mlib::queue::CurrentOptions::None)
            .await
            .with_context(|| format!("[{player}] fetching current in queue"))?;
        let queue_size = player
            .queue_size()
            .await
            .with_context(|| format!("[{player}] fetching queue size"))?;

        let last_queue = player
            .last_queue()
            .await
            .with_context(|| format!("[{player}] fetching last queue"))?
            .map(|l| format!(" (last queued {l})"))
            .unwrap_or_default();

        notify!(
            "{player}";
            content: " §btitle:§r {}\n §b meta:§r {:.0}% {}\n §bqueue:§r {}/{}{}",
                current.title,
                current.progress.as_ref().map(ToString::to_string).unwrap_or_else(|| String::from("none")),
                if current.playing { ">" } else { "||" },
                current.index,
                queue_size.saturating_sub(1),
                last_queue,
        );
    }
    Ok(())
}
