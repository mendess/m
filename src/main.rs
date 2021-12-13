mod arg_parse;

use anyhow::Context;
use arg_parse::{Command, New};
use mlib::{
    playlist::{self, Playlist, Song},
    socket,
    ytdl::{YtdlBuilder, util::extract_id},
};
use regex::Regex;
use std::io::ErrorKind;
use structopt::StructOpt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cmd = Command::from_args();
    match cmd {
        Command::Socket { new } => {
            if new.is_some() {
                println!("{}", socket::new().await?.display());
            } else {
                match socket::most_recent().await {
                    Ok((p, _)) => println!("{}", p.display()),
                    Err(e) if e.kind() == ErrorKind::NotFound => println!("/dev/null"),
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
        Command::New(New {
            queue,
            link,
            categories,
        }) => {
            let id = extract_id(&link).ok_or_else(|| anyhow::anyhow!("invalid link"))?;
            if Playlist::contains_song(id).await? {
                return Err(anyhow::anyhow!("Song already in playlist"));
            }
            println!("Fetching song info");
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
            println!("Song added: {}", song);
            if queue {
                todo!()
            }
        }
        _ => todo!(),
    }

    Ok(())
}
