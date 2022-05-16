use std::io::{stdout, Write};

use super::arg_parse::Amount;

use anyhow::Context;
use crossterm::{
    cursor::{self, MoveTo},
    terminal::{Clear, ClearType},
    QueueableCommand,
};
use futures_util::stream::StreamExt;
use mlib::{
    queue::{self, Queue},
    socket::{self, cmds, MpvSocket},
};
use structopt::StructOpt;

use crate::notify;

pub async fn quit() -> anyhow::Result<()> {
    fire("quit").await
}

pub async fn pause() -> anyhow::Result<()> {
    fire("cycle pause").await
}

pub async fn vu(Amount { amount }: Amount) -> anyhow::Result<()> {
    fire(format!("add volume {}", amount.unwrap_or(2))).await
}

pub async fn vd(Amount { amount }: Amount) -> anyhow::Result<()> {
    fire(format!("add volume -{}", amount.unwrap_or(2))).await
}

pub async fn toggle_video() -> anyhow::Result<()> {
    fire("cycle vid").await
}

pub async fn next_file(Amount { amount }: Amount) -> anyhow::Result<()> {
    let mut socket = MpvSocket::lattest().await?;
    for _ in 0..amount.unwrap_or(1) {
        socket.fire("playlist-next").await?;
    }
    Ok(())
}

pub async fn prev_file(Amount { amount }: Amount) -> anyhow::Result<()> {
    let mut socket = MpvSocket::lattest().await?;
    for _ in 0..amount.unwrap_or(1) {
        socket.fire("playlist-prev").await?;
    }
    Ok(())
}

pub async fn frwd(Amount { amount }: Amount) -> anyhow::Result<()> {
    fire(format!("seek {}", amount.unwrap_or(10))).await
}

pub async fn back(Amount { amount }: Amount) -> anyhow::Result<()> {
    fire(format!("seek -{}", amount.unwrap_or(10))).await
}

pub async fn next(Amount { amount }: Amount) -> anyhow::Result<()> {
    fire(format!("add chapter {}", amount.unwrap_or(1))).await
}

pub async fn prev(Amount { amount }: Amount) -> anyhow::Result<()> {
    fire(format!("add chapter -{}", amount.unwrap_or(1))).await
}

pub async fn shuffle() -> anyhow::Result<()> {
    Ok(MpvSocket::lattest()
        .await?
        .execute(cmds::QueueShuffle)
        .await?)
}

pub async fn toggle_loop() -> anyhow::Result<()> {
    let mut socket = MpvSocket::lattest().await?;
    let looping = match socket.compute(cmds::QueueIsLooping).await? {
        cmds::LoopStatus::Inf => false,
        cmds::LoopStatus::No => true,
        _ => false,
    };
    socket.execute(cmds::QueueLoop(looping)).await?;
    if looping {
        notify!("now looping");
    } else {
        notify!("not looping");
    }
    Ok(())
}

pub async fn status() -> anyhow::Result<()> {
    let all = socket::all();
    tokio::pin!(all);
    while let Some(mut socket) = all.next().await {
        let current = Queue::current(&mut socket)
            .await
            .with_context(|| format!("[{}] fetching current in queue", socket.path().display()))?;

        let queue_size = socket
            .compute(socket::cmds::QueueSize)
            .await
            .with_context(|| format!("[{}] fetching queue size", socket.path().display()))?;

        let last_queue = queue::last::fetch(&socket)
            .await
            .with_context(|| format!("[{}] fetching last queue", socket.path().display()))?;

        notify!(
            "Player @ {}", socket.path().display();
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

async fn fire<S: AsRef<[u8]>>(c: S) -> anyhow::Result<()> {
    Ok(MpvSocket::lattest().await?.fire(c).await?)
}
