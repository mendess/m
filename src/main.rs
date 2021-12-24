mod arg_parse;
mod notify;
mod selector;
mod session_kind;

use anyhow::Context;
use arg_parse::{Amount, Command, New, Play};
use futures_util::StreamExt;
use mlib::{
    downloaded::{check_cache, clean_downloads},
    playlist::{self, Playlist, PlaylistIds, Song},
    queue::Queue,
    socket::{cmds as sock_cmds, MpvSocket},
    ytdl::{get_playlist_video_ids, util::extract_id, YtdlBuilder},
    Error as SockErr, Link,
};
use regex::Regex;
use structopt::StructOpt;
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufWriter},
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
            add_song(&mut link, categories).await?;
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
                add_song(&mut link, categories.clone()).await?;
            }
            if queue {
                todo!()
            }
        }
        Command::Current { link, notify } => {
            let mut socket = MpvSocket::lattest().await?;
            if link {
                match Queue::link(&mut socket).await? {
                    Some(link) => {
                        notify!("{}", link);
                        return Ok(());
                    }
                    None => return Err(anyhow::anyhow!("failed to get link of current song")),
                }
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
                    format!("| {} |", current.categories.join(" | "))
                },
                if let Some(next) = current.next {
                    format!("\n\n=== UP NEXT ===\n{}", next)
                } else {
                    String::new()
                }
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
        Command::Play(Play { search, what }) => match Link::from_url(what) {
            Ok(link) => println!("{:?}", check_cache(link).await),
            Err(what) => todo!("not a link: {}", what),
        },
        _ => todo!(),
    }

    Ok(())
}

pub fn init_logger() {
    LogTracer::init().expect("Failed to set logger");

    let env_filter = if let Ok(e) = EnvFilter::try_from_default_env() {
        e
    } else {
        return;
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

async fn add_song(link: &mut String, categories: Vec<String>) -> anyhow::Result<()> {
    let b = YtdlBuilder::new(link)
        .get_title()
        .get_duration()
        .request()
        .await?;

    let song = Song {
        time: b.duration().as_secs(),
        link: if is_short_link(link) {
            std::mem::take(link)
        } else {
            format!("https://youtu.be/{}", b.id())
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
