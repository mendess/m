use std::io::{stdout, Write};

use super::arg_parse::Amount;

use anyhow::Context;
use crossterm::{
    cursor::{self, MoveTo},
    terminal::{Clear, ClearType},
    QueueableCommand,
};
use mlib::{
    players::{self, PlayerIndex},
    queue::Queue,
};
use structopt::StructOpt;

use crate::{chosen_index, notify};

pub async fn quit() -> anyhow::Result<()> {
    Ok(players::quit(chosen_index()).await?)
}

pub async fn pause() -> anyhow::Result<()> {
    Ok(players::cycle_pause(chosen_index()).await?)
}

pub async fn vu(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(players::change_volume(chosen_index(), amount.unwrap_or(2)).await?)
}

pub async fn vd(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(players::change_volume(chosen_index(), -amount.unwrap_or(2)).await?)
}

pub async fn toggle_video() -> anyhow::Result<()> {
    Ok(players::toggle_video(chosen_index()).await?)
}

pub async fn next_file(Amount { amount }: Amount) -> anyhow::Result<()> {
    let player = players::get(chosen_index());
    for _ in 0..amount.unwrap_or(1) {
        tracing::debug!("going to next file");
        player.change_file(players::Direction::Next).await?;
    }
    Ok(())
}

pub async fn prev_file(Amount { amount }: Amount) -> anyhow::Result<()> {
    let player = players::get(chosen_index());
    for _ in 0..amount.unwrap_or(1) {
        player.change_file(players::Direction::Prev).await?;
    }
    Ok(())
}

pub async fn frwd(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(players::seek(chosen_index(), amount.unwrap_or(10) as f64).await?)
}

pub async fn back(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(players::seek(chosen_index(), -(amount.unwrap_or(10) as f64)).await?)
}

pub async fn next(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(players::change_chapter(
        chosen_index(),
        players::Direction::Next,
        amount.unwrap_or(1),
    )
    .await?)
}

pub async fn prev(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(players::change_chapter(
        chosen_index(),
        players::Direction::Prev,
        amount.unwrap_or(1),
    )
    .await?)
}

pub async fn shuffle() -> anyhow::Result<()> {
    Ok(players::queue_shuffle(PlayerIndex::CURRENT).await?)
}

pub async fn toggle_loop() -> anyhow::Result<()> {
    let player = players::get(chosen_index());
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
    for index in all {
        let current = Queue::current(index)
            .await
            .with_context(|| format!("[{}] fetching current in queue", index))?;
        let player = players::get(index);
        let queue_size = player
            .queue_size()
            .await
            .with_context(|| format!("[{}] fetching queue size", index))?;

        let last_queue = player
            .last_queue()
            .await
            .with_context(|| format!("[{}] fetching last queue", index))?;

        notify!(
            "{}", index;
            content: " §btitle:§r {}\n §b meta:§r {:.0}% {}\n §bqueue:§r {}/{}{}",
                current.title,
                current.progress.as_ref().map(ToString::to_string).unwrap_or_else(|| String::from("none")),
                if current.playing { ">" } else { "||" },
                current.index,
                queue_size.saturating_sub(1),
                match last_queue {
                    Some(last_queue) => format!(" (last queued {})", last_queue),
                    None => String::new(),
                }
        );
    }
    Ok(())
}

pub async fn interactive() -> anyhow::Result<()> {
    use crate::util::RawMode;
    use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};

    let _guard = RawMode::enable()?;
    let (column, row) = cursor::position()?;
    let show_current = || async {
        let r = stdout()
            .lock()
            .queue(MoveTo(column, row))
            .and_then(|s| s.queue(Clear(ClearType::FromCursorDown)))
            .and_then(|s| s.flush());
        match r {
            Ok(_) => {
                super::process_cmd(crate::arg_parse::Command::Current {
                    notify: false,
                    link: false,
                })
                .await
            }
            Err(e) => Err(e.into()),
        }
    };
    let mut error = None;
    loop {
        show_current().await?;
        if let Some(cmd) = error.take() {
            crate::error!("invalid command: {}", cmd);
        }
        let cmd = event::read()?;
        match cmd {
            Event::Key(
                KeyEvent {
                    code: KeyCode::Char('q'),
                    modifiers: KeyModifiers::NONE,
                }
                | KeyEvent {
                    code: KeyCode::Char('c' | 'd'),
                    modifiers: KeyModifiers::CONTROL,
                },
            ) => return Ok(()),
            Event::Key(KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
            }) => {
                let mut buf = [0; 4];
                let cmd = c.encode_utf8(&mut buf);
                if let Ok(cmd) = crate::arg_parse::Command::from_iter_safe(["", &*cmd]) {
                    if matches!(cmd, crate::arg_parse::Command::Current { .. }) {
                        show_current().await?;
                    } else {
                        super::process_cmd(cmd).await?
                    }
                } else {
                    error = Some(String::from(cmd));
                }
            }
            _ => {}
        }
    }
}
