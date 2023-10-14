use std::collections::HashSet;

use crate::notify;
use crate::util::selector;
use anyhow::Context;
use futures_util::TryStreamExt;
use futures_util::{future::ready, Stream};
use itertools::Itertools;
use mlib::item::link::VideoLink;
use mlib::players::PlayerLink;
use mlib::playlist::PartialSearchResult;
use mlib::Search;
use mlib::{
    playlist::{self, Playlist, PlaylistIds, Song},
    queue::Queue,
    ytdl::YtdlBuilder,
    Link,
};
use regex::Regex;

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
    for Song { name, link, .. } in playlist.songs.into_iter().filter(filter) {
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

pub async fn new(link: Link, categories: Vec<String>) -> anyhow::Result<VideoLink> {
    let link = link
        .into_video()
        .map_err(|link| anyhow::anyhow!("{} is not a video link", link))?;
    if Playlist::contains_song(link.id()).await? {
        return Err(anyhow::anyhow!("Song already in playlist"));
    }
    notify!("Fetching song info");
    add_song(link.clone(), categories.into_iter().collect()).await?;
    Ok(link)
}

pub async fn add_playlist(
    link: &Link,
    categories: Vec<String>,
) -> anyhow::Result<impl Stream<Item = anyhow::Result<VideoLink>>> {
    let link = match link.as_playlist() {
        Some(s) => s,
        None => return Err(anyhow::anyhow!("Not a playlist link")),
    };
    tracing::debug!("loading playlist ids");
    let playlist = PlaylistIds::load().await?;
    let id_stream = YtdlBuilder::new(link).request_playlist()?;
    Ok(id_stream
        .map_err(anyhow::Error::from)
        .and_then(move |b| ready(Ok((!playlist.contains(b.id().as_str()), b))))
        .try_filter_map(move |(success, b)| async move {
            if success {
                Ok(Some(VideoLink::from_id(b.id())))
            } else {
                notify!("song already in playlist");
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
    let current = Queue::link(PlayerLink::current()).await?;
    let mut playlist = Playlist::load().await?;
    let current = current
        .id()
        .ok_or_else(|| anyhow::anyhow!("current song is not identified"))?;

    let mut current = match playlist.find_song_mut(|s| s.link.id() == current) {
        Some(c) => c,
        None => return Err(anyhow::anyhow!("current song not in playlist")),
    };

    while let Some(new_cat) = selector::selector(
        current.categories.iter(),
        "Category name? (Esq to quit)",
        current.categories.len(),
    )
    .await?
    {
        if let Some(old_cat) = current.categories.push(new_cat) {
            current.categories.remove(&old_cat);
        }
    }
    playlist.save().await?;
    Ok(())
}

pub async fn delete_song(current: bool, partial_name: Vec<String>) -> anyhow::Result<()> {
    let mut playlist = Playlist::load().await?;
    let idx = if current {
        let current = Queue::link(PlayerLink::current()).await?;
        let current = current
            .id()
            .ok_or_else(|| anyhow::anyhow!("current song is not identified"))?;
        playlist.find_song_mut(|s| s.link.id() == current).into()
    } else if !partial_name.is_empty() {
        playlist.partial_name_search_mut(partial_name.iter().map(String::as_str))
    } else {
        unreachable!()
    };
    let deleted = super::handle_search_result(idx)?.delete();
    playlist.save().await?;
    notify!("song deleted"; content: "{}", deleted);
    Ok(())
}

async fn add_song(mut link: VideoLink, categories: HashSet<String>) -> anyhow::Result<()> {
    let b = YtdlBuilder::new(&link)
        .get_title()
        .get_duration()
        .request()
        .await?;
    link.shorten();
    let song = Song {
        time: b.duration().as_secs(),
        link,
        name: b.title(),
        categories: categories.into_iter().collect(),
    };
    Playlist::add_song(&song).await?;
    notify!("Song added"; content: "{}", song);
    Ok(())
}

pub(crate) async fn info(song: Vec<String>) -> anyhow::Result<()> {
    let song_iter = song
        .iter()
        .map(String::as_str)
        .flat_map(|s| s.split_whitespace());
    let playlist = playlist::Playlist::load().await?;
    let item = playlist.partial_name_search(song_iter.clone());

    match item {
        PartialSearchResult::None => {
            let vid = match VideoLink::try_from(song.into_iter().collect::<String>()) {
                Ok(l) => YtdlBuilder::new(&l).get_title().request().await?,
                Err(e) => {
                    YtdlBuilder::new(&Search::new(e))
                        .get_title()
                        .search()
                        .await?
                }
            };
            notify!(
                "song info:";
                content:
                    "§bname:§r {}\n§blink:§r http://youtu.be/{}",
                    vid.title_ref(),
                    vid.id().as_str(),
            )
        }
        PartialSearchResult::One(s) => {
            notify!(
                "song info:";
                content:
                    "§bname:§r {}\n§blink:§r {}\n§bcategories:§r {}",
                    s.name,
                    s.link,
                    s.categories.iter().format(" | ")
            );
        }
        PartialSearchResult::Many(m) => {
            notify!(
                "too many matches:";
                content: " - {}", m.iter().format("\n - ")
            );
        }
    }
    Ok(())
}
