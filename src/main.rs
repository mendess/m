mod arg_parse;

use anyhow::Context;
use arg_parse::Command;
use mlib::{
    playlist::{self, Song},
    socket,
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
            let playlist = playlist::Playlist::load()?;

            let filter = |s: &Song| match category {
                Some(ref pat) => s.categories.iter().any(|c| pat.is_match(c)),
                None => true,
            };
            for Song { name, link, .. } in playlist.0.into_iter().filter(filter) {
                println!("{} :: {}", link, name);
            }
        }
        _ => todo!(),
    }

    Ok(())
}
