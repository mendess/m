use crate::{
    arg_parse::{Amount, DeQueue, DeQueueIndex},
    notify,
};
use anyhow::Context;
use futures_util::{
    stream::{self, FuturesUnordered},
    StreamExt, TryStreamExt,
};
use itertools::Itertools;
use mlib::{
    downloaded::check_cache,
    playlist::Playlist,
    queue::{Item, Queue},
    socket::{cmds as sock_cmds, MpvSocket},
    ytdl::YtdlBuilder,
};
use std::path::PathBuf;
use std::{collections::HashSet, io::Write};
use tempfile::NamedTempFile;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufWriter},
    process::Command as Fork,
};
use tokio::{io::BufReader, process::Command};
use tokio_stream::wrappers::LinesStream;

pub async fn current(link: bool, notify: bool) -> anyhow::Result<()> {
    let mut socket = MpvSocket::lattest().await?;
    if link {
        let link = Queue::link(&mut socket).await?;
        tracing::debug!("{:?}", link);
        notify!("{}", link);
        return Ok(());
    }
    let current = Queue::current(&mut socket).await?;
    let plus = "+".repeat(current.progress as usize / 10);
    let minus = "-".repeat(10usize.saturating_sub(plus.len()));
    notify!("Now Playing";
        content: "{}\n{}🔉{:.0}% | <{}{}> {:.0}%\n\nCategories: {}{}",
        current.title,
        if current.playing { ">" } else { "||" },
        current.volume,
        plus,
        minus,
        current.progress,
        if current.categories.is_empty() {
            String::new()
        } else {
            format!("| {} |", current.categories.iter().join(" | "))
        },
        if let Some(next) = current.next {
            format!("\n\n=== UP NEXT ===\n{}", next)
        } else {
            String::new()
        }
        ; force_notify: notify
    );
    Ok(())
}

pub async fn now(Amount { amount }: Amount) -> anyhow::Result<()> {
    let mut socket = MpvSocket::lattest()
        .await
        .context("failed getting socket")?;
    Queue::now(&mut socket, amount.unwrap_or(10).abs() as _)
        .await
        .context("failed getting queue")?
        .for_each(
            |i| {
                println!("{:2}     {}", i.index, i.item);
            },
            |i| {
                println!("{:2} ==> {}", i.index, i.item);
            },
        );
    Ok(())
}

pub async fn queue(
    q: crate::arg_parse::QueueOpts,
    items: impl IntoIterator<Item = Item>,
) -> anyhow::Result<()> {
    let mut socket = match MpvSocket::lattest().await {
        Ok(sock) => sock,
        Err(mlib::Error::NoMpvInstance) => {
            return play(items, false).await;
        }
        Err(e) => return Err(e.into()),
    };
    if q.clear {
        notify!("Clearing playlist...");
        socket
            .execute(sock_cmds::QueueClear)
            .await
            .context("clearing queue")?;
    }
    if q.reset || q.clear {
        notify!("Reseting queue...");
        mlib::queue::last::reset()
            .await
            .context("resetting queue")?;
    }
    let mut n_targets = 0;
    let mut notify_tasks = FuturesUnordered::new();
    for item in items.into_iter().inspect(|_| n_targets += 1) {
        print!("Queuing song: {} ... ", item);
        std::io::stdout().flush()?;
        socket
            .execute(sock_cmds::LoadFile(&item))
            .await
            .context("loading the file")?;
        println!("success");
        let count = socket
            .compute(sock_cmds::QueueSize)
            .await
            .context("getting the queue size")?;
        let current = socket
            .compute(sock_cmds::QueuePos)
            .await
            .context("getting the queue position")?;
        let playlist_pos = if q.no_move {
            count
        } else {
            tracing::info!("moving song in queue");
            let mut target = (current + 1).clamp(0, count.saturating_sub(1));

            if let Some(last) = mlib::queue::last::fetch()
                .await
                .context("fetching the last queue position")?
            {
                if target < last {
                    target = (last + 1).clamp(0, count.saturating_sub(1));
                }
            };
            let from = count.saturating_sub(1);
            print!(
                "Moving from {} -> {} [now playing: {}] ... ",
                count, target, current
            );
            std::io::stdout().flush()?;
            socket
                .execute(sock_cmds::QueueMove { from, to: target })
                .await
                .with_context(|| format!("moving file from {} to {}", from, target))?;
            println!("succcess");
            mlib::queue::last::set(target).await?;
            target
        };
        if q.notify {
            notify_tasks.push(tokio::spawn(notify(item, current, playlist_pos)));
        }
        if q.preemptive_download {
            todo!("{}", playlist_pos);
        }
        if notify_tasks.len() > 8 {
            if let Err(e) = notify_tasks.next().await.unwrap() {
                tracing::error!("failed to notify: {:?}", e)
            }
        }
    }
    notify_tasks
        .for_each(|j| async {
            if let Err(e) = j.unwrap() {
                tracing::error!("failed to notify: {:?}", e);
            }
        })
        .await;
    if n_targets > 5 {
        mlib::queue::last::reset()
            .await
            .context("reseting last queue")?;
    }
    Ok(())
}

async fn notify(item: Item, current: usize, target: usize) -> anyhow::Result<()> {
    let img = tempfile::NamedTempFile::new()?;
    let (img_file, img_path) = img.into_parts();
    let title = match item {
        Item::Link(l) => {
            let b = YtdlBuilder::new(&l)
                .get_title()
                .get_thumbnail()
                .request()
                .await?;
            let thumb = reqwest::get(b.thumbnail()).await?;
            let mut byte_stream = thumb.bytes_stream();
            let mut img_file = BufWriter::new(File::from(img_file));
            while let Some(chunk) = byte_stream.next().await.transpose()? {
                img_file.write_all(&chunk).await?;
            }

            b.title()
        }
        Item::File(f) => {
            let mut ffmpeg = Fork::new("ffmpeg")
                .args(["-y", "-loglevel", "error", "-hide_banner", "-vsync", "2"])
                .arg("-i")
                .arg(&f)
                .args(["-frames:v", "1"])
                .arg(&img_path)
                .kill_on_drop(true)
                .spawn()?;
            let output = Fork::new("ffprobe").arg(&f).output().await?;
            let title = memchr::memmem::find(b"title", &output.stdout)
                .and_then(|idx| memchr::memmem::find(b":", &output.stdout[idx..]))
                .and_then(|idx| {
                    memchr::memmem::find(b"\n", &output.stdout[idx..])
                        .map(|end| &output.stdout[idx..end])
                        .map(|s| String::from_utf8_lossy(s).into_owned())
                })
                .unwrap_or_else(|| f.display().to_string());
            ffmpeg.wait().await?;
            title
        }
        _ => return Ok(()),
    };
    let scaled = tempfile::NamedTempFile::new()?;
    Fork::new("convert")
        .args(["-scale", "x64", "--"])
        .arg(&img_path)
        .arg(scaled.path())
        .spawn()?
        .wait()
        .await?;
    notify!(
        "Queued '{}'", title;
        content: "Current: {}\nQueue pos: {}", current, target;
        img: scaled.path();
        force_notify: true
    );

    Ok(())
}

pub async fn dequeue(d: crate::arg_parse::DeQueue) -> anyhow::Result<()> {
    let mut socket = MpvSocket::lattest().await?;
    match d {
        DeQueue::Next => {
            let next = socket.compute(sock_cmds::QueuePos).await? + 1;
            socket.execute(sock_cmds::QueueRemove(next)).await?;
        }
        DeQueue::Prev => {
            let prev = match socket.compute(sock_cmds::QueuePos).await?.checked_sub(1) {
                Some(i) => i,
                None => {
                    return Err(anyhow::anyhow!(
                        "Nothing before the first song in the queue"
                    ))
                }
            };
            socket.execute(sock_cmds::QueueRemove(prev)).await?;
        }
        DeQueue::Pop => {
            let last = match mlib::queue::last::fetch().await? {
                Some(l) => l,
                None => return Err(anyhow::anyhow!("no last queue to pop from")),
            };
            socket.execute(sock_cmds::QueueRemove(last)).await?;
        }
        DeQueue::N {
            i: DeQueueIndex(kind, n),
        } => match kind {
            crate::arg_parse::DeQueueIndexKind::Plus => {
                let current = socket.compute(sock_cmds::QueuePos).await?;
                socket.execute(sock_cmds::QueueRemove(current + n)).await?;
            }
            crate::arg_parse::DeQueueIndexKind::Minus => {
                let current = socket.compute(sock_cmds::QueuePos).await?;
                let i = current
                    .checked_sub(n)
                    .ok_or_else(|| anyhow::anyhow!("i > {}", n))?;
                socket.execute(sock_cmds::QueueRemove(i)).await?;
            }
            crate::arg_parse::DeQueueIndexKind::Exact => {
                socket.execute(sock_cmds::QueueRemove(n)).await?;
            }
        },
        DeQueue::Cat { cat } => {
            let cat = &cat;
            let playlist = Playlist::stream()
                .await?
                .filter_map(|s| async { s.ok() })
                .filter_map(
                    |s| async move { s.categories.iter().any(|c| c.contains(cat)).then(|| s) },
                )
                .map(|s| s.link.id().to_string())
                .collect::<HashSet<_>>()
                .await;
            let mut socket = MpvSocket::lattest().await?;
            let queue = Queue::load(&mut socket, None, None).await?;

            for index in queue.iter().rev().filter_map(|s| {
                s.item
                    .id()
                    .filter(|id| playlist.contains(id.as_str()))
                    .map(|_| s.index)
            }) {
                print!("removing {}... ", index);
                std::io::stdout().flush()?;
                socket.execute(sock_cmds::QueueRemove(index)).await?;
                println!(" success");
            }
        }
    }
    Ok(())
}

pub async fn dump(file: PathBuf) -> anyhow::Result<()> {
    let mut socket = MpvSocket::lattest().await?;
    let q = Queue::load(&mut socket, None, None).await?;
    let mut file = BufWriter::new(File::create(file).await?);
    for s in q.iter() {
        file.write_all(s.item.as_bytes()).await?;
        file.write_all(b"\n").await?;
    }
    file.flush().await?;
    Ok(())
}

pub async fn load(file: PathBuf) -> anyhow::Result<()> {
    let items = LinesStream::new(BufReader::new(File::open(file).await?).lines())
        .map_ok(Item::from)
        .try_collect::<Vec<_>>()
        .await?;
    queue(Default::default(), items).await?;
    Ok(())
}

pub async fn play(items: impl IntoIterator<Item = Item>, with_video: bool) -> anyhow::Result<()> {
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

    let mut items = items.into_iter().peekable();
    let first = items.by_ref().take(20);

    let mut socket = MpvSocket::new().await?;
    let mut mpv = Command::new("mpv");
    mpv.arg("--geometry=820x466");
    mpv.arg(format!("--input-ipc-server={}", socket.path().display()));
    if !with_video {
        mpv.arg("--no-video");
    }
    if first.len() > 1 {
        mpv.arg("--loop-playlist");
    }
    mpv.args(first);

    mpv.spawn()?;

    if items.peek().is_some() {
        let (file, path) = NamedTempFile::new()?.into_parts();
        let mut file = BufWriter::new(File::from_std(file));
        for i in items {
            file.write_all(i.as_bytes()).await?;
            file.write_all(b"\n").await?;
        }
        socket.execute(sock_cmds::LoadList(&path)).await?;
    }

    Ok(())
}
