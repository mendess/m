use crate::util::{self, prompt};
use crate::{error, notify};
use anyhow::{Context, bail, ensure};
use futures_util::{Stream, future::ready};
use futures_util::{StreamExt, TryStreamExt};
use itertools::Itertools;
use levenshtein::levenshtein;
use mlib::Item;
use mlib::item::link::VideoLink;
use mlib::players::PlayerLink;
use mlib::playlist::PartialSearchResult;
use mlib::playlist::uniq_vec::UniqVec;
use mlib::{
    Link,
    playlist::{self, Playlist, PlaylistIds, Song},
    queue::Queue,
    ytdl::YtdlBuilder,
};
use regex::Regex;
use std::collections::{BinaryHeap, HashSet};

pub async fn songs(category: Option<String>) -> anyhow::Result<()> {
    let category = category
        .as_deref()
        .map(Regex::new)
        .transpose()
        .context("Invalid category pattern")?;
    let playlist = playlist::Playlist::load().await?;

    let filter = |s: &Song| match category {
        Some(ref pat) => s.all_categories().any(|c| pat.is_match(c)),
        None => true,
    };
    for Song { name, link, .. } in playlist.songs.into_iter().filter(filter) {
        println!("{link} :: {name}");
    }
    Ok(())
}

pub async fn ls_categories(free_categories: bool) -> anyhow::Result<()> {
    let playlist = Playlist::load().await?;
    let mut cat = if free_categories {
        playlist.free_categories()
    } else {
        playlist.categories()
    }
    .into_iter()
    .collect::<Vec<_>>();
    cat.sort_unstable_by_key(|(_, count)| *count);
    for (c, count) in cat {
        println!("{count:5}  {c}");
    }
    Ok(())
}

fn make_song(
    mut link: VideoLink,
    SongInfo { time, name }: SongInfo,
    SongMetadata {
        artist,
        genre,
        language,
        recommended_by,
    }: SongMetadata,
    categories: UniqVec<String>,
) -> Song {
    link.shorten();
    Song {
        name,
        link,
        time,
        categories,
        artist,
        genre,
        language,
        recommended_by,
        liked_by: vec![],
    }
}

pub async fn new(link: Link) -> anyhow::Result<VideoLink> {
    let mut categories = UniqVec::<String>::new();
    let link = link
        .into_video()
        .map_err(|link| anyhow::anyhow!("{} is not a video link", link))?;
    if Playlist::contains_song(link.id()).await? {
        return Err(anyhow::anyhow!("Song already in playlist"));
    }
    let meta = category_prompt(&mut categories).await?;
    notify!("Fetching song info");
    let song_info = fetch_song_info(&link).await?;
    let song = make_song(link.clone(), song_info, meta, categories);
    Playlist::add_song(&song).await?;
    notify!("Song added"; content: "{}", song);
    Ok(link)
}

pub async fn add_playlist(
    link: &Link,
    mut categories: UniqVec<String>,
) -> anyhow::Result<impl Stream<Item = anyhow::Result<VideoLink>> + use<>> {
    let meta = category_prompt(&mut categories).await?;
    let link = match link.as_playlist() {
        Some(s) => s,
        None => return Err(anyhow::anyhow!("Not a playlist link")),
    };
    tracing::debug!("loading playlist ids");
    let playlist = PlaylistIds::load().await?;
    let id_stream = YtdlBuilder::new(link).request_playlist().await?;
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
            let categories = categories.clone(); // TODO this clone should not be needed
            let meta = meta.clone();
            async {
                let song_info = fetch_song_info(&link).await?;
                Playlist::add_song(&make_song(link.clone(), song_info, meta, categories)).await?;
                Ok(link)
            }
        }))
}

#[derive(Debug, Default, Clone)]
struct SongMetadata {
    pub artist: Option<String>,
    pub genre: Option<String>,
    pub language: Option<String>,
    pub recommended_by: Option<String>,
}

async fn category_prompt(categories: &mut UniqVec<String>) -> anyhow::Result<SongMetadata> {
    let playlist = Playlist::load().await?;
    let playlist_categories = playlist.categories();
    let mut meta = SongMetadata::default();
    let did_you_mean_check = async |cat: &mut String| {
        if !playlist_categories.contains_key(cat.as_str()) {
            let close_matches = playlist_categories
                .iter()
                .filter(|(c, _)| levenshtein(c, cat) < 4)
                .map(|(c, _)| *c)
                .collect::<Vec<_>>();
            if !close_matches.is_empty() {
                println!("You said {cat}. Did you mean any of these?");
                if let Some(index) = util::prompt::interative_select(&close_matches, []).await? {
                    *cat = close_matches[index].to_owned();
                }
            }
        }
        anyhow::Ok(())
    };
    let mut set = async |mut value, at: &mut Option<String>| {
        did_you_mean_check(&mut value).await?;
        categories.retain(|s| *s != value);
        *at = Some(value);
        anyhow::Ok(())
    };
    let playlist = Playlist::load().await;
    if let Some(artist) = prompt::prompt("artist").await? {
        if let Ok(playlist) = &playlist {
            let genres = playlist
                .songs
                .iter()
                .filter(|s| s.artist.as_ref() == Some(&artist))
                .filter_map(|g| g.genre.as_ref())
                .collect::<HashSet<_>>();
            if !genres.is_empty() {
                println!("this artist usually plays: {genres:?}");
            }
        }
        set(artist, &mut meta.artist).await?;
    }
    if let Some(genre) = prompt::prompt("genre").await? {
        set(genre, &mut meta.genre).await?;
    }
    if let Some(language) = prompt::prompt("language").await? {
        set(language, &mut meta.language).await?;
    }
    if let Some(recomended_by) = prompt::prompt("recommended by").await? {
        set(recomended_by, &mut meta.recommended_by).await?;
    }
    if let Some(playlist) = playlist.ok().filter(|_| categories.is_empty()) {
        let mut heap = playlist
            .free_categories()
            .iter()
            .map(|(c, count)| (*count, *c))
            .collect::<BinaryHeap<_>>();
        print!("the top most common categories are: ");
        let mut i = 0;
        while let Some((c, category)) = heap.pop() {
            i += 1;
            print!("{category} ({c})");
            if i == 10 {
                break;
            }
            print!(", ");
        }
        println!();
        while let Some(cat) = prompt::prompt("category").await? {
            categories.push(cat);
        }
    }
    for cat in &mut *categories {
        did_you_mean_check(cat).await?;
    }
    ensure!(
        !categories.is_empty()
            || [
                &meta.artist,
                &meta.genre,
                &meta.language,
                &meta.recommended_by
            ]
            .iter()
            .any(|s| s.is_some()),
        "please include at least one category or metadata"
    );
    Ok(meta)
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

struct SongInfo {
    time: u64,
    name: String,
}
async fn fetch_song_info(link: &VideoLink) -> anyhow::Result<SongInfo> {
    let b = YtdlBuilder::new(link)
        .get_title()
        .get_duration()
        .request()
        .await?;
    Ok(SongInfo {
        time: b.duration().as_secs(),
        name: b.title(),
    })
}

pub(crate) async fn info(song: Vec<String>, just_id: bool) -> anyhow::Result<()> {
    let song_iter = song
        .iter()
        .map(String::as_str)
        .flat_map(|s| s.split_whitespace());
    let playlist = playlist::Playlist::load().await?;
    let item = playlist.partial_name_search(song_iter.clone());

    match item {
        PartialSearchResult::None => {
            let print_full_info = async |vid: mlib::ytdl::Ytdl<_>| {
                notify!(
                    "song info:";
                    content:
                        "§bname:§r {}\n§blink:§r http://youtu.be/{}",
                        vid.title_ref(),
                        vid.id().as_str(),
                )
            };
            match Item::from(song.join(" ")) {
                Item::Link(Link::Video(_)) if just_id => {
                    match Item::from(song.join(" ")).id() {
                        Some(id) => println!("{}", id.as_str()),
                        None => error!("song doens't have an id"),
                    };
                }
                Item::Link(Link::Video(l)) => {
                    let vid = YtdlBuilder::new(&l).get_title().request().await?;
                    print_full_info(vid).await;
                }
                Item::Search(s) => {
                    print_full_info(YtdlBuilder::new(&s).get_title().search().await?).await;
                }
                Item::Link(Link::Playlist(l)) if just_id => {
                    let mut playlist = YtdlBuilder::new(&l).request_playlist().await?;
                    while let Some(i) = playlist.next().await {
                        println!("{}", i?.id().as_str());
                    }
                }
                Item::Link(Link::Playlist(l)) => {
                    let mut playlist = YtdlBuilder::new(&l).get_title().request_playlist().await?;
                    while let Some(i) = playlist.next().await {
                        print_full_info(i?).await;
                    }
                }
                i => {
                    let Some(id) = i.id() else {
                        bail!("info for {i} not suported");
                    };
                    if just_id {
                        print!("{}", id.as_str());
                    } else {
                        let vid = YtdlBuilder::new(&VideoLink::from_id(id))
                            .get_title()
                            .request()
                            .await?;
                        print_full_info(vid).await;
                    }
                }
            };
        }
        PartialSearchResult::One(s) => {
            if just_id {
                println!("{}", s.link.id().as_str());
                return Ok(());
            }
            notify!(
                "song info:";
                content:
                    "§bname:§r {}\n§blink:§r {}\n§bcategories:§r {}",
                    s.name,
                    s.link,
                    s.all_categories().format(" | ")
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
