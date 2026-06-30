use crate::{
    arg_parse::{Amount, DeQueue, DeQueueIndex, QueueOpts},
    chosen_index,
    download_ctl::check_cache_ref,
    notify,
    util::{
        DisplayEither, DurationFmt, dl_dir,
        prompt::{self, selector},
        with_video::with_video_env,
    },
};

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
    fmt,
    io::Write,
    path::PathBuf,
    pin::pin,
    time::Duration,
};

use anyhow::Context;
use futures_util::{
    Stream, StreamExt, TryStreamExt,
    future::ready,
    stream::{self, BoxStream, FuturesUnordered},
};
use itertools::Itertools;
use mlib::{
    Error, Link, Search, VideoId,
    item::{
        PlaylistLink,
        link::{ChannelLink, HasId as _, VideoLink},
    },
    players::{
        self, PlayerLink, SmartQueueOpts, SmartQueueSummary, error::MpvError,
    },
    playlist::Playlist,
    queue::{Current, Item, Queue},
    ytdl::YtdlBuilder,
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

pub enum CurrentDisplayMode {
    Default { short: bool },
    Link,
    LinkId,
}

pub async fn current(
    mode: CurrentDisplayMode,
    notify: bool,
) -> anyhow::Result<()> {
    let playlist = Playlist::load().await?;
    match mode {
        CurrentDisplayMode::Default { short } => {
            if short {
                let link = chosen_index();
                let mut title = link.media_title().await?;
                let filename = link.filename().await?;
                if title.is_empty() {
                    title = filename.clone();
                }
                let item = Item::from(filename);
                let artist = item.fetch_item_artist(&playlist).await;
                match artist {
                    Some(artist) => println!("{artist} - {title}"),
                    None => println!("{title}"),
                }
                Ok(())
            } else {
                let current = Queue::current(
                    &chosen_index(),
                    mlib::queue::CurrentOptions::GetNext,
                )
                .await
                .context("loading the current queue")?;

                display_current(&current, notify).await
            }
        }
        CurrentDisplayMode::Link | CurrentDisplayMode::LinkId => {
            let link = Queue::link(&chosen_index())
                .await
                .context("loading the queue to fetch the link")?;
            tracing::debug!("{:?}", link);
            match mode {
                CurrentDisplayMode::Default { .. } => unreachable!(),
                CurrentDisplayMode::Link => notify!("{}", link),
                CurrentDisplayMode::LinkId => {
                    notify!(
                        "{}",
                        link.id()
                            .ok_or_else(|| anyhow::anyhow!(
                                "no id for this video"
                            ))?
                            .as_str()
                    )
                }
            }
            Ok(())
        }
    }
}

pub struct DisplaySong<'s> {
    title: &'s str,
    artist: Option<&'s str>,
    chapter: Option<&'s str>,
    categories: &'s [String],
    playstatus: Option<DisplayPlayStatus>,
}

pub struct DisplayPlayStatus {
    playing: bool,
    volume: f64,
    progress: Option<f64>,
    playback_time: Option<Duration>,
    duration: Duration,
}

impl fmt::Display for DisplaySong<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(a) = self.artist {
            write!(f, "{a} - ")?;
        }
        match self.chapter {
            Some(c) => {
                writeln!(f, "§bVideo§r: {}\n§bSong§r:  {}", self.title, c)
            }
            None => writeln!(f, "{}", self.title),
        }?;
        if let Some(ps) = &self.playstatus {
            const PROGRESS_BAR_LEN: f64 = 11.;
            let plus = match ps.progress {
                Some(progress) => "+".repeat(
                    (progress / 100. * PROGRESS_BAR_LEN).round() as usize,
                ),
                None => "???".into(),
            };
            let minus = "-"
                .repeat((PROGRESS_BAR_LEN as usize).saturating_sub(plus.len()));
            writeln!(
                f,
                "{}🔉{:.0}% | <{}{}> {:.0}%\n          {}/{}",
                if ps.playing { ">" } else { "||" },
                ps.volume,
                plus,
                minus,
                ps.progress.as_ref().unwrap_or(&-1.0),
                ps.playback_time
                    .map(DurationFmt)
                    .map(DisplayEither::Left)
                    .unwrap_or_else(|| DisplayEither::Right(String::new())),
                DurationFmt(ps.duration),
            )?;
        }
        if !self.categories.is_empty() {
            writeln!(
                f,
                "\n| {} |",
                self.categories
                    .iter()
                    .filter(|s| Some(s.as_str()) != self.artist)
                    .join(" | ")
            )?;
        };

        Ok(())
    }
}

pub async fn display_current(
    current: &Current,
    notify: bool,
) -> anyhow::Result<()> {
    let now = DisplaySong {
        title: &current.title,
        artist: current.artist.as_deref(),
        chapter: current.chapter.as_ref().map(|(_, name)| name.as_str()),
        categories: &current.categories,
        playstatus: Some(DisplayPlayStatus {
            playing: current.playing,
            volume: current.volume,
            progress: current.progress,
            playback_time: current.playback_time,
            duration: current.duration,
        }),
    };
    notify!("Now Playing"; content: "{now}"; force_notify: notify);
    if let Some(next) = &current.next {
        notify!("Up Next";
            content: "{}",
            DisplaySong {
                title: &next.title,
                chapter: None,
                artist: next.artist.as_deref(),
                categories: &next.categories,
                playstatus: None,
            };
            force_notify: notify
        )
    }
    Ok(())
}

pub async fn now(
    Amount { amount }: Amount,
    show_files: bool,
) -> anyhow::Result<()> {
    let queue = Queue::load(
        &chosen_index(),
        amount.unwrap_or(10).unsigned_abs() as usize,
    )
    .await
    .context("failed getting queue")?;
    let current = queue.current_idx();
    let playlist = Playlist::load().await?;
    stream::iter(queue.iter())
        .map(|i| {
            debug!("translating queue item: {i:?}");
            async {
                if show_files {
                    (i.index, i.item.to_string().into(), None)
                } else {
                    (
                        i.index,
                        i.item.fetch_item_title(&playlist).await,
                        i.item.fetch_item_artist(&playlist).await,
                    )
                }
            }
        })
        .buffered(32)
        .for_each(|(index, title, artist)| async move {
            static SEPERATORS: [&str; 2] = ["   ", "==>"];
            let sep = SEPERATORS[(index == current) as usize];
            match artist {
                Some(artist) => println!("{index:2} {sep} {artist} - {title}",),
                None => println!("{index:2} {sep} {title}"),
            }
        })
        .await;
    Ok(())
}

pub async fn queue<I>(
    q: crate::arg_parse::QueueOpts,
    items: I,
) -> anyhow::Result<PlayerLink>
where
    I: IntoIterator<Item = Item>,
    I::IntoIter: ExactSizeIterator,
{
    tracing::debug!(options = ?q, "queueing songs");
    let player = match players::current().await? {
        Some(_) => chosen_index(),
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
    let mut expanded_items =
        pin!(expand_playlists(items).inspect(|_| n_targets += 1));
    let dl_dir = dl_dir().await?;
    while let Some(mut item) = expanded_items.next().await {
        check_cache_ref(&dl_dir, &mut item).await;
        print!("Queuing song: {item} ... ");
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
                "Moved from {from} -> {moved_to} [now playing: {current}] ... ",
            );
        }
        if q.notify && item_count < 30 {
            notify_tasks.push(tokio::spawn(notify(item, current, moved_to)));
        }
        if notify_tasks.len() > 8
            && let Err(e) = notify_tasks.next().await.unwrap()
        {
            tracing::error!("failed to notify: {:?}", e)
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

async fn notify(
    item: Item,
    current: usize,
    target: usize,
) -> anyhow::Result<()> {
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
                    while let Some(chunk) =
                        byte_stream.next().await.transpose()?
                    {
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
                            .request_playlist()
                            .await?
                            .next()
                            .await
                            .ok_or_else(|| {
                                anyhow::anyhow!("playlist was emtpy")
                            })??;
                        handle!(b.thumbnail(), b.title())
                    }
                    None => handle!("", String::from("url")),
                },
            }
        }
        Item::File(f) => {
            let mut ffmpeg = Fork::new("ffmpeg")
                .args([
                    "-y",
                    "-loglevel",
                    "error",
                    "-hide_banner",
                    "-vsync",
                    "2",
                ])
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
    let player = &chosen_index();
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
                    ));
                }
            };
            player.queue_remove(prev).await?;
        }
        DeQueue::Pop => {
            let last = match player.last_queue().await? {
                Some(l) => l,
                None => {
                    return Err(anyhow::anyhow!("no last queue to pop from"));
                }
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
                .filter_map(|s| async move {
                    let contains = s.all_categories().any(|c| c.contains(cat));
                    contains.then_some(s)
                })
                .map(|s| s.link.id().to_string())
                .collect::<HashSet<_>>()
                .await;
            let queue = Queue::load_full(player)
                .await
                .context("loading current queue")?;

            for index in queue.iter().rev().filter_map(|s| {
                s.item
                    .banger_id()
                    .filter(|id| playlist.contains(id.as_str()))
                    .map(|_| s.index)
            }) {
                print!("removing {index}... ");
                std::io::stdout().flush()?;
                player.queue_remove(index).await?;
                println!(" success");
            }
        }
        DeQueue::Dups => {
            let queue = Queue::load_full(player)
                .await
                .context("loading current queue")?;

            let to_id = |item: &Item| match item.id() {
                Some(id) => id.as_str().to_owned(),
                None => item.to_string(),
            };
            let mut dups =
                queue.iter().fold(HashMap::<_, u32>::new(), |mut map, s| {
                    let item = to_id(&s.item);
                    *map.entry(item).or_default() += 1;
                    map
                });

            for ident in queue.iter().rev().filter(|s| {
                let count = dups.get_mut(&to_id(&s.item)).unwrap();
                *count -= 1;
                *count > 0
            }) {
                print!(
                    "removing {} @ {}... ",
                    ident.item.as_str(),
                    ident.index
                );
                std::io::stdout().flush()?;
                player.queue_remove(ident.index).await?;
                println!(" success");
            }
        }
    }
    Ok(())
}

pub async fn dump(file: PathBuf) -> anyhow::Result<()> {
    let q = Queue::load_full(&chosen_index()).await?;
    let tempfile = tempfile::Builder::new().tempfile_in(".")?;
    let mut writer = BufWriter::new(File::create(tempfile.path()).await?);
    for s in q.iter() {
        match s.item.id() {
            Some(id) => {
                writer.write_all(id.to_link().as_str().as_bytes()).await?;
            }
            None => {
                writer.write_all(s.item.as_bytes()).await?;
            }
        }
        writer.write_all(b"\n").await?;
    }
    writer.flush().await?;
    tokio::fs::rename(tempfile.path(), file).await?;
    Ok(())
}

pub async fn load(file: PathBuf, shuf: bool) -> anyhow::Result<()> {
    let mut items =
        LinesStream::new(BufReader::new(File::open(file).await?).lines())
            .map_ok(Item::from)
            .try_collect::<Vec<_>>()
            .await?;

    if shuf {
        items.shuffle(&mut rngs::OsRng);
    }

    tracing::debug!("loading {} items", items.len());
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
    let dl_dir = dl_dir().await;
    let items = expand_playlists(items)
        .map(|mut i| async {
            if let Ok(dl_dir) = &dl_dir {
                check_cache_ref(dl_dir, &mut i).await;
            }
            i
        })
        .buffered(16)
        .collect::<Vec<_>>()
        .await;

    tracing::info!("playing {:?}", items);

    tracing::info!("pausing previous mpv instance");
    match players::pause().await {
        Err(players::Error::Mpv(MpvError::NoMpvInstance)) => {}
        Err(e) => {
            crate::error!("failed to pause previous player"; content: "{:?}", e);
        }
        Ok(_) => players::set_video(false).await?,
    }

    let index = players::create(items.iter(), with_video).await?;
    Ok(index.into())
}

pub async fn run_interactive_playlist() -> anyhow::Result<()> {
    let mode = match selector(
        [
            "All",
            "single",
            "random",
            "Category",
            #[cfg(feature = "clipboard")]
            "clipboard",
        ],
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
                Some(name) => vec![
                    playlist
                        .find_song(|s| s.name == name)
                        .map(|idx| Item::Link(idx.link.clone().into()))
                        .unwrap_or_else(|| Item::Search(Search::new(name))),
                ],
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
                playlist.categories().keys().unique(),
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
                .filter(|s| s.all_categories().contains(&category.as_str()))
                .map(|l| Item::Link(l.link.into()))
                .collect()
        }
        #[cfg(feature = "clipboard")]
        "clipboard" => {
            vec![Item::from(get_clipboard_contents()?)]
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
        chosen_index().queue_loop(true).await?;
    }
    Ok(())
}

#[cfg(feature = "clipboard")]
fn get_clipboard_contents() -> anyhow::Result<String> {
    use anyhow::bail;
    use arboard::Clipboard;
    use wl_clipboard_rs::paste::{
        ClipboardType, Error, MimeType, Seat, get_contents,
    };
    let mut clip = Clipboard::new()?;
    match clip.get().text() {
        Ok(content) => Ok(content),
        Err(_) => {
            use std::io::Read;

            let result = get_contents(
                ClipboardType::Regular,
                Seat::Unspecified,
                MimeType::Text,
            );
            match result {
                Ok((mut pipe, _)) => {
                    let mut contents = vec![];
                    pipe.read_to_end(&mut contents)?;
                    Ok(String::from_utf8_lossy(&contents).into_owned())
                }

                Err(
                    e @ (Error::NoSeats
                    | Error::ClipboardEmpty
                    | Error::NoMimeType),
                ) => {
                    bail!("clipboard is empty: {e:?}")
                }

                Err(err) => Err(err)?,
            }
        }
    }
}

fn expand_playlists<I: IntoIterator<Item = Item>>(
    items: I,
) -> impl Stream<Item = Item> {
    use mlib::ytdl::YtdlStream;

    async fn expand(
        stream: YtdlStream<Box<VideoId>>,
    ) -> Result<Option<BoxStream<'static, Item>>, Error> {
        let s = stream
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
            .map(|b| VideoLink::from_id(b.id()));
        let mut s = Box::pin(s.map(|l| Item::Link(l.into())).peekable());
        if s.as_mut().peek().await.is_none() {
            tracing::warn!("playlist is empty");
            Ok(None)
        } else {
            Ok(Some(s))
        }
    }

    async fn expand_playlist(
        l: &'_ PlaylistLink,
    ) -> Result<Option<BoxStream<'static, Item>>, Error> {
        expand(YtdlBuilder::new(l).request_playlist().await?).await
    }

    async fn expand_channel(
        l: &ChannelLink,
    ) -> Result<Option<BoxStream<'static, Item>>, Error> {
        expand(YtdlBuilder::new(l).request_channel().await?).await
    }

    fn single(l: impl Into<Item>) -> BoxStream<'static, Item> {
        Box::pin(stream::once(ready(l.into())))
    }

    stream::iter(items)
        .then(move |i| async {
            let expanded = match &i {
                Item::Link(l) => match l {
                    Link::Playlist(l) => expand_playlist(l)
                        .await
                        .map(|opt_items| {
                            opt_items.or_else(|| {
                                l.clone()
                                    .into_video_link()
                                    .map(Link::from)
                                    .map(Item::from)
                                    .map(single)
                                    .ok()
                            })
                        })
                        .ok(),
                    Link::Channel(c) => expand_channel(c).await.ok(),
                    _ => None,
                },
                _ => None,
            }
            .flatten();
            expanded.unwrap_or_else(|| single(i))
        })
        .flatten()
}

pub async fn find(query: String) -> anyhow::Result<()> {
    let query = query.to_lowercase();
    let queue = Queue::load_full(&chosen_index()).await?;
    let playlist = Playlist::load().await?;
    enum Match<'s> {
        Artist(Cow<'s, str>),
        Title(Cow<'s, str>),
        None,
    }
    let matches = futures_util::stream::iter(queue.iter().map(async |i| {
        if let Some(artist) = i.item.fetch_item_artist(&playlist).await
            && artist.to_lowercase().contains(&query)
        {
            return Match::Artist(artist);
        }
        let title = i.item.fetch_item_title(&playlist).await;
        if title.to_lowercase().contains(&query) {
            Match::Title(title)
        } else {
            Match::None
        }
    }))
    .buffered(256)
    .collect::<Vec<_>>()
    .await;

    let picks = matches.iter().zip(queue.iter()).fold(
        Vec::new(),
        |mut acc, (matches, item)| {
            match matches {
                Match::None => {}
                Match::Title(t) => {
                    acc.push((t, item));
                }
                Match::Artist(a) => {
                    if !matches!(acc.last(), Some((l, _)) if *l == a) {
                        acc.push((a, item))
                    }
                }
            }
            acc
        },
    );

    let pick = match picks.len() {
        0 => return Ok(()),
        1 => 0,
        _ => {
            let pick = prompt::selector(
                picks.iter().map(|(t, _)| t),
                "Which one?",
                picks.len(),
            )
            .await?;
            if let Some(pick) = pick {
                picks.iter().position(|(i, _)| i.as_ref() == pick).unwrap()
            } else {
                return Ok(());
            }
        }
    };

    notify!("Seeking to {}", picks[pick].0);

    let target_index = picks[pick].1.index as isize;
    let player = chosen_index();
    let queue_pos = player.queue_pos().await? as isize;
    match queue_pos - target_index {
        n @ ..0 => {
            player
                .change_file(
                    players::Direction::Next,
                    Some(n.unsigned_abs() as u64),
                )
                .await?
        }
        n @ 1.. => {
            player
                .change_file(players::Direction::Prev, Some(n as u64))
                .await?
        }
        0 => {}
    }

    Ok(())
}
