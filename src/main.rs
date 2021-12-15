mod arg_parse;
mod notify;

use anyhow::Context;
use arg_parse::{Amount, Command, New};
use mlib::{
    playlist::{self, Playlist, Song},
    socket::{cmds as sock_cmds, Error as SockErr, MpvSocket},
    ytdl::{util::extract_id, YtdlBuilder},
};
use regex::Regex;
use structopt::StructOpt;
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
            link,
            categories,
        }) => {
            let id = extract_id(&link).ok_or_else(|| anyhow::anyhow!("invalid link"))?;
            if Playlist::contains_song(id).await? {
                return Err(anyhow::anyhow!("Song already in playlist"));
            }
            notify!("Fetching song info");
            let b = YtdlBuilder::new(&link)
                .get_title()
                .get_duration()
                .request()
                .await?;

            let song = Song {
                time: b.duration().as_secs(),
                link: format!("https://youtu.be/{}", b.id()),
                name: b.title(),
                categories,
            };
            Playlist::add_song(&song).await?;
            notify!("Song added"; content: "{}", song);
            if queue {
                todo!()
            }
        }
        Command::Current { .. } => {
            println!(
                "{:?}",
                MpvSocket::lattest()
                    .await?
                    .compute(sock_cmds::Filename)
                    .await?
            );
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
        error!("{}", e);
    }
    Ok(())
}
