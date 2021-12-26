mod arg_parse;
mod notify;
mod play;
mod selector;
mod session_kind;

use std::{collections::HashSet, io::Write};

use anyhow::Context;
use arg_parse::{Amount, Command, DeleteSong, New, Play};
use futures_util::{stream::FuturesUnordered, StreamExt};
use itertools::Itertools;
use mlib::{
    downloaded::clean_downloads,
    playlist::{self, Playlist, PlaylistIds, Song},
    queue::{Item, Queue},
    socket::{cmds as sock_cmds, MpvSocket},
    ytdl::{get_playlist_video_ids, util::extract_id, YtdlBuilder},
    Error as SockErr, Link, Search,
};
use regex::Regex;
use structopt::StructOpt;
use tokio::{
    fs::File,
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufWriter},
    process::Command as Fork,
};
use tracing::dispatcher::set_global_default;
use tracing_log::LogTracer;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

async fn run() -> anyhow::Result<()> {
    let cmd = Command::from_args();
    match cmd {
        Command::Socket { new } => {
            if new.is_some() {
                println!("{}", MpvSocket::new_path().await?.display());
            } else {
                match MpvSocket::lattest().await {
                    Ok(s) => println!("{}", s.path().display()),
                    Err(SockErr::NoMpvInstance) => println!("/dev/null"),
                    Err(e) => return Err(e.into()),
                }
            }
        }
        Command::Songs { category } => {
            let category = category
                .as_deref()
                .map(Regex::new)
                .transpose()
                .context("Invalid category pattern")?;
            let playlist = playlist::Playlist::load().await?;

            let filter = |s: &Song| match category {
                Some(ref pat) => s.categories.iter().any(|c| pat.is_match(c)),
                None => true,
            };
            for Song { name, link, .. } in playlist.0.into_iter().filter(filter) {
                println!("{} :: {}", link, name);
            }
        }
        Command::Cat => {
            let playlist = Playlist::load().await?;
            let mut cat = playlist.categories().collect::<Vec<_>>();
            cat.sort_unstable_by_key(|(_, count)| *count);
            for (c, count) in cat {
                println!("{:5}  {}", count, c);
            }
        }
        Command::Quit => MpvSocket::lattest().await?.fire("quit").await?,
        Command::Pause => MpvSocket::lattest().await?.fire("cycle pause").await?,
        Command::Vu(Amount { amount }) => {
            MpvSocket::lattest()
                .await?
                .fire(format!("add volume {}", amount.unwrap_or(2)))
                .await?
        }
        Command::Vd(Amount { amount }) => {
            MpvSocket::lattest()
                .await?
                .fire(format!("add volume -{}", amount.unwrap_or(2)))
                .await?
        }
        Command::ToggleVideo => MpvSocket::lattest().await?.fire("cycle vid").await?,
        Command::NextFile(Amount { amount }) => {
            let mut socket = MpvSocket::lattest().await?;
            for _ in 0..amount.unwrap_or(1) {
                socket.fire("playlist-next").await?;
            }
        }
        Command::PrevFile(Amount { amount }) => {
            let mut socket = MpvSocket::lattest().await?;
            for _ in 0..amount.unwrap_or(1) {
                socket.fire("playlist-prev").await?;
            }
        }
        Command::Frwd(Amount { amount }) => {
            MpvSocket::lattest()
                .await?
                .fire(format!("seek {}", amount.unwrap_or(10)))
                .await?
        }
        Command::Back(Amount { amount }) => {
            MpvSocket::lattest()
                .await?
                .fire(format!("seek -{}", amount.unwrap_or(10)))
                .await?
        }
        Command::Next(Amount { amount }) => {
            MpvSocket::lattest()
                .await?
                .fire(format!("add chapter {}", amount.unwrap_or(1)))
                .await?
        }
        Command::Prev(Amount { amount }) => {
            MpvSocket::lattest()
                .await?
                .fire(format!("add chapter -{}", amount.unwrap_or(1)))
                .await?
        }
        Command::New(New {
            queue,
            mut link,
            categories,
        }) => {
            let id = extract_id(&link).ok_or_else(|| anyhow::anyhow!("invalid link"))?;
            if Playlist::contains_song(id).await? {
                return Err(anyhow::anyhow!("Song already in playlist"));
            }
            notify!("Fetching song info");
            add_song(&mut link, categories.into_iter().collect()).await?;
            if queue {
                todo!()
            }
        }
        Command::AddPlaylist(New {
            queue,
            link,
            categories,
        }) => {
            if !link.contains("playlist") {
                return Err(anyhow::anyhow!("Not a playlist link"));
            }
            tracing::trace!("loading playlist ids");
            let playlist = PlaylistIds::load().await?;
            let mut id_stream = get_playlist_video_ids(&link).await?;
            let mut id = String::new();
            while {
                id.clear();
                id_stream.read_line(&mut id).await? != 0
            } {
                let id = id.trim();
                if playlist.contains(id) {
                    notify!("song already in playlist"; content: "{}", id);
                    continue;
                }
                let mut link = format!("https://youtu.be/{}", id);
                add_song(&mut link, categories.iter().cloned().collect()).await?;
            }
            if queue {
                todo!()
            }
        }
        Command::Current { link, notify } => {
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
        }
        Command::Now(Amount { amount }) => {
            let mut socket = MpvSocket::lattest()
                .await
                .context("failed getting socket")?;
            let queue = Queue::now(&mut socket, amount.unwrap_or(10).abs() as _)
                .await
                .context("failed getting queue")?;
            for i in queue.before {
                println!("{:2}     {}", i.index, i.item);
            }
            println!("{:2} ==> {}", queue.current.index, queue.current.item);
            for i in queue.after {
                println!("{:2}     {}", i.index, i.item);
            }
        }
        Command::Shuffle => {
            MpvSocket::lattest()
                .await?
                .execute(sock_cmds::QueueShuffle)
                .await?
        }
        Command::Loop => {
            let mut socket = MpvSocket::lattest().await?;
            let looping = match socket.compute(sock_cmds::QueueIsLooping).await? {
                sock_cmds::LoopStatus::Inf => false,
                sock_cmds::LoopStatus::No => true,
                _ => false,
            };
            socket.execute(sock_cmds::QueueLoop(looping)).await?;
            if looping {
                notify!("now looping");
            } else {
                notify!("not looping");
            }
        }
        Command::CleanDownloads => {
            let ids = PlaylistIds::load().await?;
            let to_delete = clean_downloads(&ids).await?;
            tokio::pin!(to_delete);
            while let Some(f) = to_delete.next().await {
                match f {
                    Ok(f) => {
                        if let Err(e) = tokio::fs::remove_file(&f).await {
                            error!("Failed to delete {}", f.display(); content: "{}", e)
                        }
                    }
                    Err(e) => {
                        error!("something went wrong when inspecting a file"; content: "{}", e)
                    }
                }
            }
        }
        Command::Dump { file } => {
            let mut socket = MpvSocket::lattest().await?;
            let q = Queue::load(&mut socket, None, None).await?;
            let mut file = BufWriter::new(File::create(file).await?);
            for s in q.iter() {
                file.write_all(s.item.as_bytes()).await?;
                file.write_all(b"\n").await?;
            }
            file.flush().await?;
        }
        Command::Play(Play { search, what }) => {
            play::play(search_params_to_items(what, search).await?, false).await?;
        }
        Command::ChCat => {
            let mut socket = MpvSocket::lattest().await?;
            let current = Queue::link(&mut socket).await?;
            let mut playlist = Playlist::load().await?;
            let current = current
                .id()
                .ok_or_else(|| anyhow::anyhow!("current song is not identified"))?;

            let mut current = match playlist.find_song_mut(|s| s.link.id() == current) {
                Some(c) => c,
                None => return Err(anyhow::anyhow!("current song not in playlist")),
            };

            while let Some(new_cat) = selector::input("Category name? (Esq to quit)").await? {
                current.categories.insert(new_cat);
            }
            playlist.save().await?;
        }
        Command::DeleteSong(DeleteSong {
            current,
            partial_name,
        }) => {
            let mut playlist = Playlist::load().await?;
            let idx = if current {
                let mut socket = MpvSocket::lattest().await?;
                let current = Queue::link(&mut socket).await?;
                let current = current
                    .id()
                    .ok_or_else(|| anyhow::anyhow!("current song is not identified"))?;
                Ok(playlist.find_song_mut(|s| s.link.id() == current))
            } else if !partial_name.is_empty() {
                playlist.partial_name_search_mut(partial_name.iter().map(String::as_str))
            } else {
                unreachable!()
            };
            notify!("song deleted"; content: "{}", handle_search_result(idx)?.delete());
        }
        Command::Queue(q) => queue(q).await?,
        _ => todo!(),
    }

    Ok(())
}

async fn queue(q: arg_parse::Queue) -> anyhow::Result<()> {
    let items = search_params_to_items(q.play_opts.what, q.play_opts.search).await?;
    let mut socket = match MpvSocket::lattest().await {
        Ok(sock) => sock,
        Err(mlib::Error::NoMpvInstance) => {
            return play::play(items, false).await;
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
    let n_targets = items.len();
    let mut notify_tasks = FuturesUnordered::new();
    for item in items {
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
            let b = YtdlBuilder::new(l.as_str())
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

pub fn init_logger() {
    LogTracer::init().expect("Failed to set logger");

    let env_filter = if let Ok(e) = EnvFilter::try_from_default_env() {
        e
    } else {
        EnvFilter::new("warn")
    };

    let fmt = fmt::layer().event_format(fmt::format());

    let sub = Registry::default().with(env_filter).with(fmt);

    set_global_default(sub.into()).expect("Failed to set global default");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logger();
    if let Err(e) = run().await {
        error!("{:?}", e);
    }
    Ok(())
}

async fn add_song(link: &mut String, categories: HashSet<String>) -> anyhow::Result<()> {
    let b = YtdlBuilder::new(link)
        .get_title()
        .get_duration()
        .request()
        .await?;

    let song = Song {
        time: b.duration().as_secs(),
        link: if is_short_link(link) {
            Link::from_url(std::mem::take(link)).unwrap()
        } else {
            Link::from_id(b.id())
        },
        name: b.title(),
        categories,
    };
    Playlist::add_song(&song).await?;
    notify!("Song added"; content: "{}", song);
    Ok(())
}

fn is_short_link(s: &str) -> bool {
    s.starts_with("https://youtu.be/")
}

fn handle_search_result<T>(r: Result<Option<T>, usize>) -> anyhow::Result<T> {
    match r {
        Ok(Some(t)) => Ok(t),
        Ok(None) => return Err(anyhow::anyhow!("song not in playlist")),
        Err(too_many_matches) => {
            return Err(anyhow::anyhow!("too many matches: {}", too_many_matches))
        }
    }
}

#[derive(Debug)]
pub struct SongQuery {
    pub items: Vec<Item>,
    pub words: Vec<String>,
}

impl SongQuery {
    pub async fn new(strings: Vec<String>) -> Self {
        let mut items = vec![];
        let mut words = vec![];
        for x in strings {
            match Link::from_url(x) {
                Ok(l) => items.push(Item::Link(l)),
                Err(s) => match tokio::fs::metadata(&s).await {
                    Ok(_) => items.push(Item::File(s.into())),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => words.push(s),
                    Err(e) => {
                        tracing::error!("error checking if {:?} was a path to a file: {:?}", s, e)
                    }
                },
            }
        }
        Self { items, words }
    }
}

async fn search_params_to_items(strings: Vec<String>, search: bool) -> anyhow::Result<Vec<Item>> {
    let SongQuery { mut items, words } = SongQuery::new(strings).await;
    let link = if words.is_empty() {
        return Ok(items);
    } else if search {
        Item::Search(Search::new(words.join(" ")))
    } else {
        Item::Link(
            handle_search_result(
                Playlist::load()
                    .await?
                    .partial_name_search_mut(words.iter().map(String::as_str)),
            )?
            .delete()
            .link,
        )
    };
    items.push(link);
    Ok(items)
}
