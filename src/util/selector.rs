use super::session_kind::SessionKind;
use std::{
    fmt::Display,
    io::{stdout, Write},
    os::unix::prelude::ExitStatusExt,
    pin::Pin,
    process::{ExitStatus, Stdio},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    process::Command,
};

pub async fn selector<I, S>(
    items: I,
    prompt: &str,
    list_len: usize,
) -> anyhow::Result<Option<String>>
where
    S: AsRef<str>,
    I: IntoIterator<Item = S>,
{
    match SessionKind::current().await {
        SessionKind::Cli => fzf(items.into_iter(), prompt).await,
        SessionKind::Gui => {
            dmenu(
                items.into_iter(),
                prompt,
                if list_len > 80 { 30 } else { list_len },
            )
            .await
        }
    }
}

async fn fzf<I, S>(items: I, prompt: &str) -> anyhow::Result<Option<String>>
where
    S: AsRef<str>,
    I: Iterator<Item = S>,
{
    let mut command = Command::new("fzf");
    let FeedAndRead { line, status, .. } = feed_and_read(
        items,
        command.args(["-i", "--prompt", &format!("{} ", prompt), "--print-query"]),
    )
    .await?;
    match status.code() {
        Some(0 | 1) => Ok(line),
        Some(130) => Ok(None),
        Some(n) => Err(anyhow::anyhow!("process exited with status: {n}")),
        None => {
            if status.core_dumped() {
                return Err(anyhow::anyhow!("core dumped :("));
            } else if let Some(sig) = status.signal() {
                return Err(anyhow::anyhow!("killed by signal: {sig}"));
            } else {
                return Err(anyhow::anyhow!("process exited with status: {:?}", status));
            }
        }
    }
}

async fn dmenu<I, S>(items: I, prompt: &str, list_len: usize) -> anyhow::Result<Option<String>>
where
    S: AsRef<str>,
    I: Iterator<Item = S>,
{
    let mut command = Command::new("dmenu");
    let FeedAndRead { line, status, .. } = feed_and_read(
        items,
        command.args(["-i", "-p", prompt, "-l", &list_len.to_string()]),
    )
    .await?;
    if !status.success() {
        if status.core_dumped() {
            return Err(anyhow::anyhow!("core dumped :("));
        } else if let Some(sig) = status.signal() {
            return Err(anyhow::anyhow!("killed by signal: {sig}"));
        } else {
            return Err(anyhow::anyhow!(
                "process exited with status: {:?}",
                status.code()
            ));
        }
    }

    Ok(line)
}

struct FeedAndRead {
    line: Option<String>,
    status: ExitStatus,
}

async fn feed_and_read<I, S>(items: I, command: &mut Command) -> anyhow::Result<FeedAndRead>
where
    S: AsRef<str>,
    I: Iterator<Item = S>,
{
    tracing::debug!(
        "running command {:?} with args {:?}",
        command.as_std().get_program(),
        command.as_std().get_args()
    );
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut writer = BufWriter::new(child.stdin.take().unwrap());
    for i in items {
        writer.write_all(i.as_ref().as_bytes()).await?;
        writer.write_all(b"\n").await?;
    }
    writer.flush().await?;
    drop(writer);
    let mut last = None;
    let mut reader = BufReader::new(child.stdout.take().unwrap()).lines();
    while let Some(line) = reader.next_line().await? {
        last = Some(line)
    }

    Ok(FeedAndRead {
        line: last,
        status: child.wait().await?,
    })
}

type CustomKeybind<'c, E> = (
    char,
    Box<dyn for<'e> Fn(&'e E, usize) -> Pin<Box<dyn std::future::Future<Output = ()> + 'e>> + 'c>,
);

pub async fn interactive_select<E: Display, const K: usize>(
    table: &[E],
    custom_keybinds: [CustomKeybind<'_, E>; K],
) -> anyhow::Result<Option<usize>> {
    use crate::util::RawMode;
    use crossterm::{
        cursor::{self, MoveTo, MoveToNextLine},
        event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
        style::Print,
        terminal::{Clear, ClearType},
        QueueableCommand,
    };

    if SessionKind::current().await == SessionKind::Gui {
        return Err(anyhow::anyhow!(
            "interactive select only works in terminal mode"
        ));
    }

    let _raw_mode = RawMode::enable()?;
    let start_position = cursor::position()?;
    let stdout = stdout();
    let mut stdout = stdout.lock();
    let mut selected = 0;
    loop {
        stdout
            .queue(MoveTo(start_position.0, start_position.1))?
            .queue(Clear(ClearType::FromCursorDown))?;
        for (i, e) in table.iter().enumerate() {
            stdout.queue(Print(if i == selected { " ❯ " } else { "   " }))?;
            stdout.queue(Print(e))?.queue(MoveToNextLine(1))?;
        }
        stdout.flush()?;
        let e = event::read()?;
        match e {
            Event::Key(KeyEvent {
                code: KeyCode::Enter,
                ..
            }) => {
                break;
            }
            Event::Key(KeyEvent {
                code: KeyCode::Char('k'),
                ..
            }) => {
                selected = selected.saturating_sub(1);
            }
            Event::Key(KeyEvent {
                code: KeyCode::Char('j'),
                ..
            }) => {
                selected = (selected + 1).clamp(0, table.len().saturating_sub(1));
            }
            Event::Key(KeyEvent {
                code: KeyCode::Char('d'),
                modifiers: KeyModifiers::CONTROL,
            }) => return Ok(None),
            Event::Key(KeyEvent {
                code: KeyCode::Char(ch),
                ..
            }) => {
                if let Some((_, f)) = custom_keybinds.iter().find(|(c, _)| ch == *c) {
                    f(&table[selected], selected).await;
                }
            }
            _ => {}
        }
    }
    Ok(Some(selected))
}
