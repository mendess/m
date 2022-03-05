use crate::{
    arg_parse::{Amount, DeQueue, DeQueueIndex, QueueOpts},
    download_ctl::check_cache_ref,
    notify,
    util::{dl_dir, selector::selector, with_video::with_video_env},
};

use std::{
    collections::HashSet, io::Write, os::unix::prelude::ExitStatusExt, path::PathBuf, pin::Pin,
    process::Stdio, time::Duration,
};

use anyhow::Context;
use futures_util::{
    future::ready,
    stream::{self, FuturesUnordered},
    Stream, StreamExt, TryFutureExt, TryStreamExt,
};
use itertools::Itertools;
use mlib::{
    item::{link::VideoLink, PlaylistLink},
    playlist::Playlist,
    queue::{Item, Queue},
    socket::{cmds as sock_cmds, MpvSocket},
    ytdl::YtdlBuilder,
    Error, Search,
};
use rand::{prelude::SliceRandom, rngs};
use serde::Deserialize;
use tempfile::NamedTempFile;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufWriter},
    process::Command as Fork,
    sync::OnceCell,
    time::sleep,
};
use tokio::{io::BufReader, process::Command};
use tokio_stream::wrappers::LinesStream;

pub async fn current(link: bool, notify: bool) -> anyhow::Result<()> {
    let mut socket = MpvSocket::current().await.context("connecting to socket")?;
    if link {
        let link = Queue::link(&mut socket)
            .await
            .context("loading the queue to fetch the link")?;
        tracing::debug!("{:?}", link);
        notify!("{}", link);
        return Ok(());
    }
    let current = Queue::current(&mut socket)
        .await
        .context("loading the current queue")?;
    let plus = match current.progress {
        Some(progress) => "+".repeat(progress as usize / 10),
        None => "???".into(),
    };
    let minus = "-".repeat(10usize.saturating_sub(plus.len()));
    let song = match current.chapter {
        Some(c) => {
            format!("§bVideo§r: {}\n§bSong§r:  {}", current.title, c)
        }
        None => current.title,
    };
    notify!("Now Playing";
        content: "{}\n{}🔉{:.0}% | <{}{}> {:.0}%{}{}",
        song,
        if current.playing { ">" } else { "||" },
        current.volume,
        plus,
        minus,
        current.progress.as_ref().map(ToString::to_string).unwrap_or_else(|| String::from("none")),
        if current.categories.is_empty() {
            String::new()
        } else {
            format!("\n\nCategories: | {} |", current.categories.iter().join(" | "))
        },
        if let Some(next) = current.next {
            let up_next = match VideoLink::from_url(next) {
                Ok(l) => resolve_link(&l).await,
                Err(next) => mlib::item::clean_up_path(&next).unwrap_or(&next).to_owned(),
            };
            format!("\n\n=== UP NEXT ===\n{up_next}")
        } else {
            String::new()
        };
        force_notify: notify
    );
    Ok(())
}

pub async fn resolve_link(link: &VideoLink) -> String {
    static LIST: OnceCell<Result<Playlist, mlib::Error>> = OnceCell::const_new();
    let name = match LIST.get_or_init(Playlist::load).await {
        Ok(list) => Ok(list.find_by_link(link).await.map(|s| s.name.clone())),
        Err(e) => Err(e),
    };
    match name {
        Ok(Some(name)) => name,
        _ => match YtdlBuilder::new(link).get_title().request().await {
            Ok(r) => r.title(),
            Err(_) => link.to_string(),
        },
    }
}

pub async fn now(Amount { amount }: Amount) -> anyhow::Result<()> {
    let mut socket = MpvSocket::current()
        .await
        .context("failed getting socket")?;
    let queue = Queue::now(&mut socket, amount.unwrap_or(10).abs() as _)
        .await
        .context("failed getting queue")?;
    let current = queue.current_idx();
    stream::iter(queue.iter())
        .map(|i| async {
            let s = match &i.item {
                // TODO: should be able to move here
                Item::Link(l) => match l.as_video() {
                    Ok(l) => resolve_link(l).await,
                    Err(_) => l.to_string(),
                },
                Item::File(f) => mlib::item::clean_up_path(&f)
                    .map(ToString::to_string)
                    .unwrap_or_else(|| f.to_string_lossy().into_owned()),
                Item::Search(s) => YtdlBuilder::new(s)
                    .get_title()
                    .search()
                    .await
                    .map(|b| b.title())
                    .unwrap_or_else(|l| l.to_string()),
            };
            (i.index, s)
        })
        .buffered(8)
        .for_each(|(index, s)| async move {
            static SEPERATORS: [&str; 2] = ["   ", "==>"];
            println!(
                "{:2} {} {}",
                index,
                SEPERATORS[(index == current) as usize],
                s
            )
        })
        .await;
    Ok(())
}

pub async fn queue(
    q: crate::arg_parse::QueueOpts,
    items: impl IntoIterator<Item = Item>,
) -> anyhow::Result<()> {
    let mut socket = match MpvSocket::current().await {
        Ok(sock) => sock,
        Err(mlib::Error::NoMpvInstance) => {
            tracing::debug!("no mpv instance, starting a new one");
            return play(items, with_video_env()).await;
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
        socket
            .last()
            .and_then(|mut s| async move { s.reset().await })
            .await
            .context("resetting queue")?;
    }
    let mut n_targets = 0;
    let mut notify_tasks = FuturesUnordered::new();
    let mut items = expand_playlists(items).inspect(|_| n_targets += 1);
    while let Some(mut item) = items.next().await {
        check_cache_ref(dl_dir()?, &mut item).await;
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
            // TODO: this entire logic needs some refactoring
            // there are a lot of edge cases
            // - the queue might have shrunk since the last time we queued
            // - the queue might have looped around

            tracing::debug!("current position: {}", current);
            let mut target = (current + 1) % count;
            tracing::debug!("first target: {}", target);

            if let Some(last) = socket
                .last()
                .and_then(|mut s| async move { s.fetch().await })
                .await
                .context("fetching the last queue position")?
            {
                tracing::debug!("last: {}", last);
                if target <= last {
                    target = (last + 1) % count;
                    tracing::debug!("second target: {}", target);
                }
            };
            let from = count.saturating_sub(1);
            if from != target {
                print!(
                    "Moving from {} -> {} [now playing: {}] ... ",
                    from, target, current
                );
                std::io::stdout().flush()?;
                socket
                    .execute(sock_cmds::QueueMove { from, to: target })
                    .await
                    .with_context(|| format!("moving file from {} to {}", from, target))?;
                println!("succcess");
            }
            socket
                .last()
                .and_then(|mut s| async move { s.set(target).await })
                .await?;
            target
        };
        if q.notify {
            notify_tasks.push(tokio::spawn(notify(item, current, playlist_pos)));
        }
        if q.preemptive_download {
            todo!(
                "preemptive_download download is not implemented {}",
                playlist_pos
            );
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
        tracing::debug!("reseting queue because got {} targets", n_targets);
        socket
            .last()
            .and_then(|mut s| async move { s.reset().await })
            .await
            .context("reseting last queue")?;
    }
    Ok(())
}

async fn notify(item: Item, current: usize, target: usize) -> anyhow::Result<()> {
    let img = tempfile::NamedTempFile::new()?;
    let (img_file, img_path) = img.into_parts();
    tracing::debug!("image tmp path: {}", img_path.display());
    let title = match item {
        Item::Link(l) => {
            let b = match l.into_video() {
                Ok(v) => {
                    YtdlBuilder::new(&v)
                        .get_title()
                        .get_thumbnail()
                        .request()
                        .await?
                }
                Err(pl) => YtdlBuilder::new(&pl)
                    .get_title()
                    .get_thumbnail()
                    .request_playlist()?
                    .next()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("playlist was emtpy"))??,
            };
            tracing::debug!("thumbnail: {}", b.thumbnail());
            let thumb = reqwest::get(b.thumbnail()).await?;
            let mut byte_stream = thumb.bytes_stream();
            let mut img_file = BufWriter::new(File::from(img_file));
            while let Some(chunk) = byte_stream.next().await.transpose()? {
                img_file.write_all(&chunk).await?;
            }
            img_file.flush().await?;

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
            #[derive(Deserialize)]
            struct GetTitle {
                format: Format,
            }
            #[derive(Deserialize)]
            struct Format {
                tags: Tags,
            }
            #[derive(Deserialize)]
            struct Tags {
                title: String,
            }
            let output = Fork::new("ffprobe")
                .arg(&f)
                .args(["-v", "quiet", "-show_format", "-print_format", "json"])
                .output()
                .await?;
            let title = serde_json::from_slice::<GetTitle>(&output.stdout)?
                .format
                .tags
                .title;

            ffmpeg.wait().await?;
            title
        }
        _ => return Ok(()),
    };
    let scaled = tempfile::NamedTempFile::new()?;
    tracing::debug!("image scaled tmp path: {}", scaled.path().display());
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
    let mut socket = MpvSocket::current().await?;
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
            let last = match socket
                .last()
                .and_then(|mut s| async move { s.fetch().await })
                .await?
            {
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
                .await
                .context("getting playlist file")?
                .filter_map(|s| async { s.ok() })
                .filter_map(
                    |s| async move { s.categories.iter().any(|c| c.contains(cat)).then(|| s) },
                )
                .map(|s| s.link.id().to_string())
                .collect::<HashSet<_>>()
                .await;

            let mut socket = MpvSocket::current().await?;
            let queue = Queue::load(&mut socket, None, None)
                .await
                .context("loading current queue")?;

            for index in queue.iter().rev().filter_map(|s| {
                s.item
                    .id()
                    .filter(|id| playlist.contains(id.as_str()))
                    .map(|_| s.index)
            }) {
                print!("removing {}... ", index);
                std::io::stdout().flush()?;
                socket
                    .execute(sock_cmds::QueueRemove(index))
                    .await
                    .context("removing from queue")?;
                println!(" success");
            }
        }
    }
    Ok(())
}

pub async fn dump(file: PathBuf) -> anyhow::Result<()> {
    let mut socket = MpvSocket::current().await?;
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
    let mut items = items.into_iter().collect::<Vec<_>>();
    tracing::info!("playing {:?}", items);
    // let to_download = stream::iter(items.iter_mut())
    //     .then(|i| async { check_cache_ref(dl_dir().ok()?, i).await })
    //     .buffered(16)
    //     .await;
    stream::iter(items.iter_mut())
        .for_each_concurrent(16, |i| async {
            let dl_dir = match dl_dir() {
                Ok(d) => d,
                Err(_) => return,
            };
            check_cache_ref(dl_dir, i).await
        })
        .await;

    let mut items = items.into_iter().peekable();
    let first = items.by_ref().take(20);

    if let Ok(mut socket) = MpvSocket::current().await {
        tracing::info!("pausing previous mpv instance");
        if let Err(e) = socket.execute(sock_cmds::Pause).await {
            crate::error!("failed to pause previous player"; content: "{:?}", e);
        }
    }
    let mut unconn_socket = MpvSocket::new_unconnected()
        .await
        .context("creating a new socket")?;
    tracing::debug!(?unconn_socket, "created a new unconnected socket");

    let mut mpv = Command::new("mpv");
    mpv.args(["--geometry=820x466", "--no-terminal"]);
    mpv.arg(format!(
        "--input-ipc-server={}",
        unconn_socket.path().display()
    ));
    if !with_video {
        mpv.arg("--no-video");
    }
    if first.len() > 1 {
        mpv.arg("--loop-playlist");
    }
    mpv.args(first);
    mpv.stdout(Stdio::null());

    tracing::debug!(args = ?mpv.as_std().get_args(), "spawning new mpv process");
    let mut child = mpv.spawn().context("spawning mpv")?;
    tracing::debug!(?child, "new mpv process spawned");

    if items.peek().is_some() {
        tracing::info!("queueing the leftover songs");
        let mut connected = false;
        for i in 0..5 {
            tracing::debug!("attempt {}", i + 1);
            match unconn_socket.connect().await {
                Err((_, s)) => {
                    unconn_socket = s;
                    sleep(Duration::from_secs(i * 2)).await;
                }
                Ok(mut socket) => {
                    let (file, path) = NamedTempFile::new()?.into_parts();
                    let mut file = BufWriter::new(File::from_std(file));
                    for i in items {
                        file.write_all(i.as_bytes())
                            .await
                            .context("writing bytes")?;
                        file.write_all(b"\n").await.context("writing bytes")?;
                    }
                    file.flush().await?;
                    socket.execute(sock_cmds::LoadList(&path)).await?;
                    connected = true;
                    break;
                }
            };
        }
        if !connected {
            crate::error!("never managed to connect to queue the rest of the songs")
        }
    }
    match tokio::time::timeout(Duration::from_secs(1), child.wait()).await {
        Ok(status) => {
            let status = status?;
            if !status.success() {
                crate::error!(
                    "Error spawning mpv";
                    content: "Exit code: {:?}, was killed: {:?}",
                    status.code(),
                    status.signal()
                );
            }
        }
        Err(_elapsed) => {
            // success probably
        }
    }

    Ok(())
}

pub async fn run_interactive_playlist() -> anyhow::Result<()> {
    let mode = match selector(
        ["All", "single", "random", "Category", "clipboard"],
        "Mode?",
        5,
    )
    .await?
    {
        Some(m) => m,
        None => return Ok(()),
    };

    let playlist = Playlist::load().await.context("loading playlist")?;

    let mut vids = match mode.as_str() {
        "single" => {
            let song_name = selector(
                playlist.0.iter().rev().map(|s| &s.name),
                "Which video?",
                playlist.0.len(),
            )
            .await?;
            match song_name {
                None => return Ok(()),
                Some(name) => vec![playlist
                    .find_song(|s| s.name == name)
                    .map(|idx| Item::Link(idx.link.clone().into()))
                    .unwrap_or_else(|| Item::Search(Search::new(name)))],
            }
        }
        "random" => match playlist.0.choose(&mut rngs::OsRng) {
            Some(x) => {
                vec![Item::Link(x.link.clone().into())]
            }
            None => return Err(anyhow::anyhow!("empty playlist")),
        },
        "All" => playlist
            .0
            .into_iter()
            .rev()
            .map(|l| Item::Link(l.link.into()))
            .collect(),
        "Category" => {
            let category = selector(
                playlist.categories().map(|(s, _)| s).unique(),
                "Which category?",
                30,
            )
            .await?;
            let category = match category {
                Some(c) => c,
                None => return Ok(()),
            };
            playlist
                .0
                .into_iter()
                .filter(|s| s.categories.contains(&category))
                .map(|l| Item::Link(l.link.into()))
                .collect()
        }
        "clipboard" => {
            use clipboard::{ClipboardContext, ClipboardProvider};
            let clipboard = ClipboardContext::new()
                .and_then(|mut c| c.get_contents())
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            vec![Item::from(clipboard)]
        }
        _ => return Ok(()),
    };

    vids = expand_playlists(vids).collect().await;

    let loop_list = vids.len() > 1;
    if loop_list {
        vids.shuffle(&mut rngs::OsRng);
    }

    queue(
        QueueOpts {
            notify: true,
            no_move: mode == "All",
            ..Default::default()
        },
        vids,
    )
    .await
    .context("queueing")?;
    if loop_list {
        MpvSocket::current()
            .await?
            .execute(sock_cmds::QueueLoop(true))
            .await?
    }
    Ok(())
}

fn expand_playlists<I: IntoIterator<Item = Item>>(items: I) -> impl Stream<Item = Item> {
    let expand_playlist = |l: &'_ PlaylistLink| {
        Result::<_, Error>::Ok(
            YtdlBuilder::new(l)
                .request_playlist()?
                .filter_map(|r| async {
                    match r {
                        Ok(x) => Some(x),
                        Err(e) => {
                            crate::error!(
                                "failed to parse playlist item when expanding playlist: {:?}",
                                e
                            );
                            None
                        }
                    }
                })
                .map(|b| VideoLink::from_id(b.id())),
        )
    };

    stream::iter(items).flat_map(move |i| match i {
        Item::Link(mut l) => {
            if let Some(playlist) = l.as_playlist_mut() {
                match expand_playlist(playlist) {
                    Ok(s) => Box::pin(s.map(|l| Item::Link(l.into()))),
                    Err(_) => Box::pin(stream::once(ready(Item::Link(l))))
                        as Pin<Box<dyn Stream<Item = Item>>>,
                }
            } else {
                Box::pin(stream::once(ready(Item::Link(l))))
            }
        }
        x => Box::pin(stream::once(ready(x))),
    })
}
