use std::collections::HashSet;

use crate::notify;
use crate::util::selector;
use anyhow::Context;
use futures_util::TryStreamExt;
use futures_util::{future::ready, Stream};
use mlib::{
    playlist::{self, Playlist, PlaylistIds, Song},
    queue::Queue,
    socket::MpvSocket,
    ytdl::{get_playlist_video_ids, YtdlBuilder},
    Link, LinkId,
};
use regex::Regex;
use tokio::io::AsyncBufReadExt;
use tokio_stream::wrappers::LinesStream;

pub async fn songs(category: Option<String>) -> anyhow::Result<()> {
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
    Ok(())
}

pub async fn cat() -> anyhow::Result<()> {
    let playlist = Playlist::load().await?;
    let mut cat = playlist.categories().collect::<Vec<_>>();
    cat.sort_unstable_by_key(|(_, count)| *count);
    for (c, count) in cat {
        println!("{:5}  {}", count, c);
    }
    Ok(())
}

pub async fn new(link: String, categories: Vec<String>) -> anyhow::Result<Link> {
    let link = match Link::from_url(link) {
        Ok(l) => l,
        Err(_) => return Err(anyhow::anyhow!("not a link")),
    };
    let id = link.id();
    if Playlist::contains_song(id).await? {
        return Err(anyhow::anyhow!("Song already in playlist"));
    }
    notify!("Fetching song info");
    add_song(link.clone(), categories.into_iter().collect()).await?;
    Ok(link)
}

pub async fn add_playlist(
    link: String,
    categories: Vec<String>,
) -> anyhow::Result<impl Stream<Item = anyhow::Result<Link>>> {
    if !link.contains("playlist") {
        return Err(anyhow::anyhow!("Not a playlist link"));
    }
    tracing::trace!("loading playlist ids");
    let playlist = PlaylistIds::load().await?;
    let id_stream = get_playlist_video_ids(&link).await?;
    Ok(LinesStream::new(id_stream.stdout.lines())
        .map_err(anyhow::Error::from)
        .map_ok(|mut id| {
            id.retain(|c| !c.is_whitespace());
            id
        })
        .and_then(move |id| ready(Ok((playlist.contains(&id), id))))
        .try_filter_map(move |(success, id)| async move {
            if success {
                Ok(Some(Link::from_id(LinkId::new_unchecked(&id))))
            } else {
                notify!("song already in playlist"; content: "{}", ";");
                Ok(None)
            }
        })
        .and_then(move |link| {
            let categories = categories.iter().cloned().collect();
            async {
                add_song(link.clone(), categories).await?;
                Ok(link)
            }
        }))
}

pub async fn ch_cat() -> anyhow::Result<()> {
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
    Ok(())
}

pub async fn delete_song(current: bool, partial_name: Vec<String>) -> anyhow::Result<()> {
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
    notify!("song deleted"; content: "{}", super::handle_search_result(idx)?.delete());
    Ok(())
}

async fn add_song(link: Link, categories: HashSet<String>) -> anyhow::Result<()> {
    let b = YtdlBuilder::new(&link)
        .get_title()
        .get_duration()
        .request()
        .await?;

    let song = Song {
        time: b.duration().as_secs(),
        link,
        name: b.title(),
        categories,
    };
    Playlist::add_song(&song).await?;
    notify!("Song added"; content: "{}", song);
    Ok(())
}
