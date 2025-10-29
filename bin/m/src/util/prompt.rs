use super::session_kind::SessionKind;
use crate::error;
use crate::util::RawMode;
use anyhow::{Context as _, bail};
use crossterm::{
    QueueableCommand,
    cursor::{MoveTo, MoveToNextLine},
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    style::Print,
    terminal::{Clear, ClearType},
};
use futures_util::future::BoxFuture;
use rustyline::error::ReadlineError;
use std::{
    fmt::{Display, Write as _},
    io::{Write as _, stdout},
    os::unix::prelude::ExitStatusExt,
    process::{ExitStatus, Stdio},
    str::FromStr,
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

pub async fn prompt_with_default<T>(prompt: &str, default: &str) -> anyhow::Result<Option<T>>
where
    T: FromStr,
    T::Err: Display,
    anyhow::Error: From<T::Err>,
{
    prompt_validated_with_default(prompt, |_| Ok(()), default).await
}

pub async fn prompt_validated_with_default<T>(
    prompt: &str,
    validation: impl FnMut(&T) -> Result<(), String>,
    default: &str,
) -> anyhow::Result<Option<T>>
where
    T: FromStr,
    T::Err: Display,
    anyhow::Error: From<T::Err>,
{
    impl_prompt_validated(prompt, validation, Some(default)).await
}

pub async fn prompt<T>(prompt: &str) -> anyhow::Result<Option<T>>
where
    T: FromStr,
    T::Err: Display,
    anyhow::Error: From<T::Err>,
{
    prompt_validated(prompt, |_| Ok(())).await
}

pub async fn prompt_validated<T>(
    prompt: &str,
    validation: impl FnMut(&T) -> Result<(), String>,
) -> anyhow::Result<Option<T>>
where
    T: FromStr,
    T::Err: Display,
    anyhow::Error: From<T::Err>,
{
    impl_prompt_validated(prompt, validation, None).await
}

pub async fn impl_prompt_validated<T>(
    prompt: &str,
    mut validation: impl FnMut(&T) -> Result<(), String>,
    default: Option<&str>,
) -> anyhow::Result<Option<T>>
where
    T: FromStr,
    T::Err: Display,
    anyhow::Error: From<T::Err>,
{
    match SessionKind::current().await {
        SessionKind::Cli => {
            let mut editor = rustyline::DefaultEditor::new()?;
            let mut rl_prompt = format!("{prompt}: ");
            loop {
                match editor.readline_with_initial(&rl_prompt, (default.unwrap_or_default(), "")) {
                    Ok(input) => match (!input.is_empty()).then(|| input.parse()) {
                        Some(Ok(t)) => match validation(&t) {
                            Ok(()) => return Ok(Some(t)),
                            Err(reason) => {
                                rl_prompt.clear();
                                writeln!(rl_prompt, "error: {reason}. {prompt}: ").unwrap();
                            }
                        },
                        Some(Err(e)) => {
                            rl_prompt.clear();
                            writeln!(rl_prompt, "error: {e}. {prompt}: ").unwrap();
                        }
                        None => return Ok(None),
                    },
                    Err(ReadlineError::Eof) => return Ok(None),
                    Err(ReadlineError::Interrupted) => bail!("canceled"),
                    Err(ReadlineError::WindowResized) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
        SessionKind::Gui => loop {
            match dmenu(std::iter::empty::<&str>(), prompt, 1).await {
                Ok(Some(r)) if r.is_empty() => return Ok(None),
                Ok(Some(r)) => match r.parse() {
                    Ok(t) => match validation(&t) {
                        Ok(()) => return Ok(Some(t)),
                        Err(reason) => error!("invalid response"; content: "{reason}"),
                    },
                    Err(e) => error!("invalid response"; content: "{e}"),
                },
                Ok(None) => error!("please respond"),
                Err(e) => error!("error reading user response"; content: "{e}"),
            }
        },
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
        command.args(["-i", "--prompt", &format!("{prompt} "), "--print-query"]),
    )
    .await?;
    match status.code() {
        Some(0 | 1) => Ok(line),
        Some(130) => Ok(None),
        Some(n) => Err(anyhow::anyhow!("process exited with status: {n}")),
        None => {
            if status.core_dumped() {
                Err(anyhow::anyhow!("core dumped :("))
            } else if let Some(sig) = status.signal() {
                Err(anyhow::anyhow!("killed by signal: {sig}"))
            } else {
                Err(anyhow::anyhow!("process exited with status: {:?}", status))
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

pub struct CustomKeybind<'c, E> {
    pub key: char,
    pub action: &'c dyn Fn(&E, usize) -> BoxFuture<'_, ()>,
}

pub async fn interative_select<E: Display, const K: usize>(
    table: &[E],
    custom_keybinds: [CustomKeybind<'_, E>; K],
) -> anyhow::Result<Option<usize>> {
    if SessionKind::current().await == SessionKind::Gui {
        return Err(anyhow::anyhow!(
            "interative select only works in terminal mode"
        ));
    }

    let raw_mode = RawMode::enable()?;
    let stdout = stdout();
    let mut stdout = stdout.lock();
    let mut selected = 0;

    let start_position = raw_mode.guarantee_space(
        &mut stdout,
        u16::try_from(table.len()).context("too many items in the input table")?,
    )?;

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
            Event::Key(
                KeyEvent {
                    code: KeyCode::Char('d'),
                    modifiers: KeyModifiers::CONTROL,
                    ..
                }
                | KeyEvent {
                    code: KeyCode::Esc, ..
                },
            ) => return Ok(None),
            Event::Key(KeyEvent {
                code: KeyCode::Char(ch),
                ..
            }) => {
                if let Some(ckey) = custom_keybinds.iter().find(|ckey| ch == ckey.key) {
                    (ckey.action)(&table[selected], selected).await;
                }
            }
            _ => {}
        }
    }
    Ok(Some(selected))
}
