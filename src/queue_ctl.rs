use crate::{
    arg_parse::{Amount, DeQueue, DeQueueIndex, QueueOpts},
    download_ctl::check_cache_ref,
    notify,
    util::{dl_dir, selector::selector, with_video::with_video_env, DisplayEither, DurationFmt},
};

use std::{collections::HashSet, io::Write, path::PathBuf, pin::Pin};

use anyhow::Context;
use futures_util::{
    future::ready,
    stream::{self, FuturesUnordered},
    Stream, StreamExt, TryStreamExt,
};
use itertools::Itertools;
use mlib::{
    item::{link::VideoLink, PlaylistLink},
    players::{self, error::MpvError, PlayerLink, SmartQueueOpts, SmartQueueSummary},
    playlist::Playlist,
    queue::{Current, Item, Queue},
    ytdl::YtdlBuilder,
    Error, Search,
};
use rand::{prelude::SliceRandom, rngs};
use serde::Deserialize;
use tokio::io::BufReader;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufWriter},
    process::Command as Fork,
};
use tokio_stream::wrappers::LinesStream;
use tracing::debug;

pub async fn current(link: bool, notify: bool) -> anyhow::Result<()> {
    if link {
        let link = Queue::link(PlayerLink::current())
            .await
            .context("loading the queue to fetch the link")?;
        tracing::debug!("{:?}", link);
        notify!("{}", link);
        return Ok(());
    }
    let current = Queue::current(PlayerLink::current())
        .await
        .context("loading the current queue")?;

    display_current(&current, notify).await
}

pub async fn display_current(current: &Current, notify: bool) -> anyhow::Result<()> {
    const PROGRESS_BAR_LEN: f64 = 11.;
    let plus = match current.progress {
        Some(progress) => "+".repeat((progress / 100. * PROGRESS_BAR_LEN).round() as usize),
        None => "???".into(),
    };
    let minus = "-".repeat((PROGRESS_BAR_LEN as usize).saturating_sub(plus.len()));
    let song = match &current.chapter {
        Some(c) => {
            format!("§bVideo§r: {}\n§bSong§r:  {}", current.title, c.1)
        }
        None => current.title.clone(),
    };
    let current_categories = if current.categories.is_empty() {
        String::new()
    } else {
        format!("\n\n| {} |", current.categories.iter().join(" | "))
    };
    let up_next = if let Some(next) = current.next.clone() {
        format!("\n\n=== UP NEXT ===\n{next}")
    } else {
        String::new()
    };
    notify!("Now Playing";
        content: "{}\n{}🔉{:.0}% | <{}{}> {:.0}%\n          {}/{}{}{}",
        song,
        if current.playing { ">" } else { "||" },
        current.volume,
        plus,
        minus,
        current.progress.as_ref().unwrap_or(&-1.0),
        current
            .playback_time
            .map(DurationFmt)
            .map(DisplayEither::Left)
            .unwrap_or_else(|| DisplayEither::Right(String::new())),
        DurationFmt(current.duration),
        current_categories,
        up_next;
        force_notify: notify
    );
    Ok(())
}

pub async fn now(Amount { amount }: Amount) -> anyhow::Result<()> {
    let queue = Queue::load(
        PlayerLink::current(),
        amount.unwrap_or(10).unsigned_abs() as usize,
    )
    .await
    .context("failed getting queue")?;
    let current = queue.current_idx();
    stream::iter(queue.iter())
        .map(|i| {
            debug!("translating queue item: {i:?}");
            async { (i.index, i.item.fetch_item_title().await) }
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

pub async fn queue<I>(q: crate::arg_parse::QueueOpts, items: I) -> anyhow::Result<PlayerLink>
where
    I: IntoIterator<Item = Item>,
    I::IntoIter: ExactSizeIterator,
{
    let player = match players::current().await? {
        Some(index) => PlayerLink::of(index),
        None => {
            tracing::debug!("no mpv instance, starting a new one");
            return play(items, with_video_env()).await;
        }
    };
    tracing::debug!("found a player: {player:?}");
    if q.clear {
        notify!("Clearing playlist...");
        player.queue_clear().await.context("clearing queue")?;
    }
    if q.reset || q.clear {
        notify!("Reseting queue...");
        player.last_queue_clear().await.context("resetting queue")?;
    }
    let mut n_targets = 0;
    let mut notify_tasks = FuturesUnordered::new();
    let items = items.into_iter();
    let item_count = items.len();
    let mut expanded_items = expand_playlists(items).inspect(|_| n_targets += 1);
    while let Some(mut item) = expanded_items.next().await {
        check_cache_ref(dl_dir().await?, &mut item).await;
        print!("Queuing song: {} ... ", item);
        std::io::stdout().flush()?;
        let SmartQueueSummary {
            from,
            moved_to,
            current,
        } = player
            .smart_queue(item.clone(), SmartQueueOpts { no_move: q.no_move })
            .await
            .context("when queueing")?;

        if from != moved_to {
            println!("success");
            println!(
                "Moved from {} -> {} [now playing: {}] ... ",
                from, moved_to, current
            );
        }
        if q.notify && item_count < 30 {
            notify_tasks.push(tokio::spawn(notify(item, current, moved_to)));
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
        tracing::info!("reseting queue because got {} targets", n_targets);
        player
            .last_queue_clear()
            .await
            .context("reseting last queue")?;
    }
    Ok(player)
}

async fn notify(item: Item, current: usize, target: usize) -> anyhow::Result<()> {
    let img = tempfile::Builder::new().suffix(".png").tempfile()?;
    let (img_file, img_path) = img.into_parts();
    tracing::debug!("image tmp path: {}", img_path.display());
    let title = match item {
        Item::Link(l) => {
            macro_rules! handle {
                ($thumbnail:expr, $title:expr) => {{
                    let thumbnail = $thumbnail;
                    tracing::debug!("thumbnail: {}", thumbnail);
                    let thumb = reqwest::get(thumbnail).await?;
                    let mut byte_stream = thumb.bytes_stream();
                    let mut img_file = BufWriter::new(File::from(img_file));
                    while let Some(chunk) = byte_stream.next().await.transpose()? {
                        img_file.write_all(&chunk).await?;
                    }
                    img_file.flush().await?;

                    $title
                }};
            }
            match l.into_video() {
                Ok(v) => {
                    let b = YtdlBuilder::new(&v)
                        .get_title()
                        .get_thumbnail()
                        .request()
                        .await?;
                    handle!(b.thumbnail(), b.title())
                }
                Err(pl) => match pl.as_playlist() {
                    Some(pl) => {
                        let b = YtdlBuilder::new(pl)
                            .get_title()
                            .get_thumbnail()
                            .request_playlist()?
                            .next()
                            .await
                            .ok_or_else(|| anyhow::anyhow!("playlist was emtpy"))??;
                        handle!(b.thumbnail(), b.title())
                    }
                    None => handle!("", String::from("url")),
                },
            }
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
    let player = PlayerLink::current();
    match d {
        DeQueue::Next => {
            player.queue_remove(player.queue_pos().await? + 1).await?;
        }
        DeQueue::Prev => {
            let prev = match player.queue_pos().await?.checked_sub(1) {
                Some(i) => i,
                None => {
                    return Err(anyhow::anyhow!(
                        "Nothing before the first song in the queue"
                    ))
                }
            };
            player.queue_remove(prev).await?;
        }
        DeQueue::Pop => {
            let last = match player.last_queue().await? {
                Some(l) => l,
                None => return Err(anyhow::anyhow!("no last queue to pop from")),
            };
            player.queue_remove(last).await?;
        }
        DeQueue::Current => {
            let to_remove = player.queue_pos().await?;
            player.queue_remove(to_remove).await?;
        }
        DeQueue::N {
            i: DeQueueIndex(kind, n),
        } => {
            let to_remove = match kind {
                crate::arg_parse::DeQueueIndexKind::Plus => {
                    let current = player.queue_pos().await?;
                    current + n
                }
                crate::arg_parse::DeQueueIndexKind::Minus => {
                    let current = player.queue_pos().await?;
                    current
                        .checked_sub(n)
                        .ok_or_else(|| anyhow::anyhow!("i > {}", n))?
                }
                crate::arg_parse::DeQueueIndexKind::Exact => n,
            };
            player.queue_remove(to_remove).await?;
        }
        DeQueue::Cat { cat } => {
            let cat = &cat;
            let playlist = Playlist::stream()
                .await
                .context("getting playlist file")?
                .filter_map(|s| async { s.ok() })
                .filter_map(
                    |s| async move { s.categories.iter().any(|c| c.contains(cat)).then_some(s) },
                )
                .map(|s| s.link.id().to_string())
                .collect::<HashSet<_>>()
                .await;
            let queue = Queue::load_full(player)
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
                player.queue_remove(index).await?;
                println!(" success");
            }
        }
    }
    Ok(())
}

pub async fn dump(file: PathBuf) -> anyhow::Result<()> {
    let q = Queue::load_full(PlayerLink::current()).await?;
    let mut file = BufWriter::new(File::create(file).await?);
    for s in q.iter() {
        file.write_all(s.item.as_bytes()).await?;
        file.write_all(b"\n").await?;
    }
    file.flush().await?;
    Ok(())
}

pub async fn load(file: PathBuf, shuf: bool) -> anyhow::Result<()> {
    let mut items = LinesStream::new(BufReader::new(File::open(file).await?).lines())
        .map_ok(Item::from)
        .try_collect::<Vec<_>>()
        .await?;

    if shuf {
        items.shuffle(&mut rngs::OsRng);
    }

    queue(Default::default(), items)
        .await?
        .queue_loop(true)
        .await?;

    Ok(())
}

pub async fn play(
    items: impl IntoIterator<Item = Item>,
    with_video: bool,
) -> anyhow::Result<PlayerLink> {
    let mut items = items.into_iter().collect::<Vec<_>>();
    tracing::info!("playing {:?}", items);
    stream::iter(items.iter_mut())
        .for_each_concurrent(16, |i| async {
            let dl_dir = match dl_dir().await {
                Ok(d) => d,
                Err(_) => return,
            };
            check_cache_ref(dl_dir, i).await
        })
        .await;

    tracing::info!("pausing previous mpv instance");
    match players::pause().await {
        Err(players::Error::Mpv(MpvError::NoMpvInstance)) => {}
        Err(e) => {
            crate::error!("failed to pause previous player"; content: "{:?}", e);
        }
        Ok(_) => {}
    }

    let index = players::create(items.iter(), with_video).await?;
    Ok(index.into())
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
                playlist.songs.iter().rev().map(|s| &s.name),
                "Which video?",
                playlist.songs.len(),
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
        "random" => match playlist.songs.choose(&mut rngs::OsRng) {
            Some(x) => {
                vec![Item::Link(x.link.clone().into())]
            }
            None => return Err(anyhow::anyhow!("empty playlist")),
        },
        "All" => playlist
            .songs
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
                .songs
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
        players::queue_loop(true).await?;
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
