mod interactive;

pub use interactive::interactive;

use super::arg_parse::Amount;

use anyhow::Context;
use mlib::{players, queue::Queue};

use crate::{chosen_index, notify, util::DurationFmt};
use futures_util::StreamExt as _;
use std::time::Duration;

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

pub async fn volume() -> anyhow::Result<()> {
    let volume = chosen_index().volume().await?;
    println!("{volume}");
    Ok(())
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
    tracing::debug!("going to next file");
    player
        .change_file(
            players::Direction::Next,
            amount.map(|a| u64::try_from(a).unwrap_or(0)),
        )
        .await?;
    Ok(())
}

pub async fn prev_file<A>(amount: A) -> anyhow::Result<()>
where
    A: Into<Amount>,
{
    let Amount { amount } = amount.into();
    let player = chosen_index();
    player
        .change_file(
            players::Direction::Prev,
            amount.map(|a| u64::try_from(a).unwrap_or(0)),
        )
        .await?;
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
    Ok(chosen_index().queue_shuffle().await?)
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

pub async fn set_looping(state: bool) -> anyhow::Result<()> {
    let player = chosen_index();
    player.queue_loop(state).await?;
    if state {
        notify!("now looping");
    } else {
        notify!("not looping");
    }
    Ok(())
}

pub async fn status() -> anyhow::Result<()> {
    let all = players::all().await?;
    for player in all {
        let queue = Queue::load_full(&player)
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

        let current = queue
            .current_song(&player, mlib::queue::CurrentOptions::None)
            .await?;
        let total_runtime = futures_util::stream::iter(queue.iter())
            .map(|s| async {
                match s.item.runtime().await {
                    Ok(d) => d,
                    Err(e) => {
                        tracing::warn!(
                            ?e,
                            "failed to get runtime for {}",
                            s.item
                        );
                        Duration::ZERO
                    }
                }
            })
            .buffer_unordered(32)
            .fold(Duration::ZERO, |acc, i| std::future::ready(acc + i))
            .await;

        notify!(
            "{player}";
            content: " §btitle:§r {}{}\n §b meta:§r {:.0}% {}\n §bqueue:§r {}/{}{}\n §btotal runtime:§r {}",
                current.artist.map(|a| format!("{a} - ")).unwrap_or_default(),
                current.title,
                current.progress.as_ref().map(ToString::to_string).unwrap_or_else(|| String::from("none")),
                if current.playing { ">" } else { "||" },
                current.index,
                queue_size.saturating_sub(1),
                last_queue,
                DurationFmt(total_runtime),
        );
    }
    Ok(())
}
