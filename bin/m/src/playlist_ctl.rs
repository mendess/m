use crate::notify;
use crate::util::{self, prompt};
use anyhow::{Context, bail, ensure};
use futures_util::{StreamExt, TryStreamExt as _};
use itertools::Itertools;
use levenshtein::levenshtein;
use mlib::Item;
use mlib::item::ItemId;
use mlib::item::link::{BangerLink, HasId as _, VideoLink};
use mlib::players::PlayerLink;
use mlib::playlist::PartialSearchResult;
use mlib::playlist::uniq_vec::UniqVec;
use mlib::{
    Link,
    playlist::{self, Playlist, Song},
    queue::Queue,
    ytdl::YtdlBuilder,
};
use regex::Regex;
use std::collections::{BinaryHeap, HashMap, HashSet};

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
    link: BangerLink,
    SongInfo { time, name }: SongInfo,
    SongMetadata {
        artist,
        genre,
        language,
        recommended_by,
    }: SongMetadata,
    categories: UniqVec<String>,
) -> Song {
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

pub async fn new(
    links: &[BangerLink],
    base_meta: SongMetadata,
    mut categories: UniqVec<String>,
    batch: bool,
) -> anyhow::Result<()> {
    if futures_util::stream::iter(links.iter())
        .then(|l| Playlist::contains_song(l.id()))
        .try_all(|b| async move { b })
        .await?
    {
        return Err(anyhow::anyhow!("Song already in playlist"));
    }
    let meta = if batch {
        base_meta
    } else {
        category_prompt(&mut categories).await?.merge(base_meta)
    };
    for link in links {
        notify!("Fetching song info");
        let song_info = fetch_song_info(link).await?;
        let song = make_song(link.clone(), song_info, meta.clone(), categories.clone());
        Playlist::add_song(&song).await?;
        notify!("Song added"; content: "{}", song);
    }
    Ok(())
}

#[derive(Debug, Default, Clone, clap::Parser, serde::Serialize, serde::Deserialize)]
pub struct SongMetadata {
    #[arg(long)]
    pub artist: Option<String>,
    #[arg(long)]
    pub genre: Option<String>,
    #[arg(long)]
    pub language: Option<String>,
    #[arg(long)]
    pub recommended_by: Option<String>,
}

impl SongMetadata {
    fn merge(self, other: Self) -> Self {
        Self {
            artist: self.artist.or(other.artist),
            genre: self.genre.or(other.genre),
            language: self.language.or(other.language),
            recommended_by: self.recommended_by.or(other.recommended_by),
        }
    }
}

async fn free_category_prompt(
    playlist: &Playlist,
    new_categories: &mut UniqVec<String>,
) -> anyhow::Result<()> {
    let playlist_categories = playlist.free_categories();
    let mut heap = playlist_categories
        .iter()
        .map(|(c, count)| (*count, *c))
        .collect::<BinaryHeap<_>>();
    println!("the top most common categories are: ");
    let mut i = 0;
    while let Some((c, category)) = heap.pop() {
        i += 1;
        println!("\t{category} ({c})");
        if i == 10 {
            break;
        }
    }
    println!();
    if !new_categories.is_empty() {
        println!("current categories:");
        for c in new_categories.iter() {
            println!("\t{c}");
        }
    }
    while let Some(cat) = prompt::prompt("category").await? {
        new_categories.push(cat);
    }
    for cat in &mut *new_categories {
        did_you_mean_check(&playlist_categories, cat).await?;
    }
    Ok(())
}

async fn did_you_mean_check<S>(
    playlist_categories: &HashMap<S, usize>,
    cat: &mut String,
) -> anyhow::Result<()>
where
    S: AsRef<str> + Eq + std::hash::Hash,
    S: std::borrow::Borrow<str>,
{
    if !playlist_categories.contains_key(cat) {
        let close_matches = playlist_categories
            .iter()
            .filter(|(c, _)| levenshtein(c.as_ref(), cat) < 4)
            .map(|(c, _)| c.as_ref())
            .collect::<Vec<_>>();
        if !close_matches.is_empty() {
            println!("You said {cat}. Did you mean any of these?");
            if let Some(index) = util::prompt::interative_select(&close_matches, []).await? {
                *cat = close_matches[index].to_owned();
            }
        }
    }
    Ok(())
}

async fn set<S>(
    playlist_categories: &HashMap<S, usize>,
    new_categories: &mut UniqVec<String>,
    mut value: String,
    at: &mut Option<String>,
) -> anyhow::Result<()>
where
    S: AsRef<str> + Eq + std::hash::Hash,
    S: std::borrow::Borrow<str>,
{
    did_you_mean_check(playlist_categories, &mut value).await?;
    new_categories.retain(|s| *s != value);
    *at = Some(value);
    Ok(())
}

async fn category_prompt(new_categories: &mut UniqVec<String>) -> anyhow::Result<SongMetadata> {
    let playlist = Playlist::load().await?;
    let playlist_categories = playlist.categories();
    let mut meta = SongMetadata::default();
    if let Some(artist) = prompt::prompt("artist").await? {
        let genres = playlist
            .songs
            .iter()
            .filter(|s| s.artist.as_ref() == Some(&artist))
            .filter_map(|g| g.genre.as_ref())
            .collect::<HashSet<_>>();
        if !genres.is_empty() {
            println!("this artist usually plays: {genres:?}");
        }
        set(
            &playlist_categories,
            new_categories,
            artist,
            &mut meta.artist,
        )
        .await?;
    }
    if let Some(genre) = prompt::prompt("genre").await? {
        set(&playlist_categories, new_categories, genre, &mut meta.genre).await?;
    }
    if let Some(language) = prompt::prompt("language").await? {
        set(
            &playlist_categories,
            new_categories,
            language,
            &mut meta.language,
        )
        .await?;
    }
    if let Some(recomended_by) = prompt::prompt("recommended by").await? {
        set(
            &playlist_categories,
            new_categories,
            recomended_by,
            &mut meta.recommended_by,
        )
        .await?;
    }
    free_category_prompt(&playlist, new_categories).await?;
    ensure!(
        !new_categories.is_empty()
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
            .banger_id()
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
async fn fetch_song_info(link: &BangerLink) -> anyhow::Result<SongInfo> {
    let meta = link.metadata().await?;
    Ok(SongInfo {
        name: meta.title,
        time: meta.duration.as_secs(),
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
                Item::Link(Link::Video(l)) if just_id => {
                    println!("{}", l.id().as_str());
                }
                Item::Link(Link::Video(l)) => {
                    let vid = YtdlBuilder::new(&l).get_title().request().await?;
                    print_full_info(vid).await;
                }
                Item::Link(Link::Banger(l)) if just_id => {
                    println!("{}", l.id().as_str());
                }
                Item::Link(Link::Banger(l)) => {
                    if let Some(song) = playlist.find_by_link(&l) {
                        notify!(
                            "song info:";
                            content:
                                "§bname:§r {}\n§blink:§r {}",
                                song.name,
                                l,
                        )
                    };
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
                    } else if let ItemId::VideoId(v) = id {
                        let vid = YtdlBuilder::new(&VideoLink::from_id(v))
                            .get_title()
                            .request()
                            .await?;
                        print_full_info(vid).await;
                    } else {
                        bail!("idk men, too many enums")
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

pub async fn add_category(mut playlist: Playlist, song: usize) -> anyhow::Result<()> {
    let all_categories = playlist.categories_owned();
    macro_rules! edit {
        ($field:ident) => {
            let new_f = match &playlist.songs[song].$field {
                Some(f) => prompt::prompt_with_default(stringify!($field), f).await?,
                None => prompt::prompt(stringify!($field)).await?,
            };
            if let Some(new_f) = new_f {
                let song = &mut playlist.songs[song];
                set(
                    &all_categories,
                    &mut song.categories,
                    new_f,
                    &mut song.$field,
                )
                .await?;
            }
        };
    }
    edit!(artist);
    edit!(genre);
    edit!(recommended_by);
    edit!(language);

    let mut categories = std::mem::take(&mut playlist.songs[song].categories);
    free_category_prompt(&playlist, &mut categories).await?;
    playlist.songs[song].categories = categories;

    notify!("saving edited song as"; content: "{}", playlist.songs[song]);
    playlist.save().await?;
    Ok(())
}

pub async fn delete_category(mut playlist: Playlist, song: usize) -> anyhow::Result<()> {
    while let Some(delete) = prompt::interative_select(&playlist.songs[song].liked_by, []).await? {
        playlist.songs[song].liked_by.remove(delete);
    }
    while let Some(delete) = prompt::interative_select(&playlist.songs[song].categories, []).await?
    {
        playlist.songs[song].categories.remove_at(delete);
    }
    playlist.save().await?;
    Ok(())
}
