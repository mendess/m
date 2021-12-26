use crate::session_kind::SessionKind;
use std::process::Stdio;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    process::Command,
};

pub async fn input(prompt: &str) -> anyhow::Result<Option<String>> {
    selector::<_, &str>([], prompt, 0).await
}

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
    feed_and_read(
        items,
        command
            .arg("-i")
            .arg("--prompt")
            .arg(prompt)
            .arg("--print-query"),
    )
    .await
}

async fn dmenu<I, S>(items: I, prompt: &str, list_len: usize) -> anyhow::Result<Option<String>>
where
    S: AsRef<str>,
    I: Iterator<Item = S>,
{
    let mut command = Command::new("dmenu");
    feed_and_read(
        items,
        command
            .arg("-i")
            .arg("-p")
            .arg(prompt)
            .arg("-l")
            .arg(&list_len.to_string()),
    )
    .await
}

async fn feed_and_read<I, S>(items: I, command: &mut Command) -> anyhow::Result<Option<String>>
where
    S: AsRef<str>,
    I: Iterator<Item = S>,
{
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

    Ok(last)
}
