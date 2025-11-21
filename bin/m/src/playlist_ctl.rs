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
use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::str::FromStr;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename = "snake_case")]
pub enum CategoryType {
    Free,
    Artist,
    Genres,
    Language,
    RecommendedBy,
    LikedBy,
}

impl FromStr for CategoryType {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "free" => Ok(Self::Free),
            "artist" => Ok(Self::Artist),
            "genres" => Ok(Self::Genres),
            "language" => Ok(Self::Language),
            "recommended_by" => Ok(Self::RecommendedBy),
            "liked_by" => Ok(Self::LikedBy),
            _ => Err(format!("invalid type: {s:?}")),
        }
    }
}

pub async fn ls_categories(kind: Option<CategoryType>) -> anyhow::Result<()> {
    let playlist = Playlist::load().await?;
    let mut cat = match kind {
        Some(CategoryType::Free) => playlist.free_categories(),
        Some(CategoryType::Artist) => {
            playlist.categories_of_kind(|s| s.artist.as_deref().into_iter())
        }
        Some(CategoryType::Genres) => {
            playlist.categories_of_kind(|s| s.genres.iter().map(|s| s.as_str()))
        }
        Some(CategoryType::Language) => {
            playlist.categories_of_kind(|s| s.language.as_deref().into_iter())
        }
        Some(CategoryType::RecommendedBy) => {
            playlist.categories_of_kind(|s| s.recommended_by.as_deref().into_iter())
        }
        Some(CategoryType::LikedBy) => {
            playlist.categories_of_kind(|s| s.liked_by.iter().map(|s| s.as_str()))
        }
        None => playlist.categories(),
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
        genres,
        language,
        recommended_by,
        liked_by,
    }: SongMetadata,
    categories: UniqVec<String>,
) -> Song {
    Song {
        name,
        link,
        time,
        categories,
        artist,
        #[expect(deprecated)]
        _genre: genres.first().cloned(),
        genres: genres.into(),
        language,
        recommended_by,
        liked_by: liked_by.into(),
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
        category_prompt(&mut categories, base_meta).await?
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
    #[arg(long, required = false)]
    pub genres: Vec<String>,
    #[arg(long)]
    pub language: Option<String>,
    #[arg(long)]
    pub recommended_by: Option<String>,
    #[arg(long, required = false)]
    pub liked_by: Vec<String>,
}

const LIST_PROMPT_CATEGORIES: (&str, &str) = ("category", "categories");
const LIST_PROMPT_GENRE: (&str, &str) = ("genre", "genres");
const LIST_PROMPT_LIKED_BY: (&str, &str) = ("liked by", "likers");

trait VecLike<T> {
    fn is_empty(&self) -> bool;
    fn push(&mut self, t: T);
    fn iter<'s>(&'s self) -> std::slice::Iter<'s, T>;
}

macro_rules! impl_vec_like {
    ($type:ident) => {
        impl<T> VecLike<T> for $type<T>
        where
            T: PartialEq,
        {
            fn is_empty(&self) -> bool {
                self.is_empty()
            }

            fn iter<'s>(&'s self) -> std::slice::Iter<'s, T> {
                self.as_slice().iter()
            }

            fn push(&mut self, t: T) {
                $type::<T>::push(self, t);
            }
        }
    };
}

impl_vec_like!(Vec);
impl_vec_like!(UniqVec);

async fn list_prompt<'v, V>(
    (singular, plural): (&str, &str),
    comparison_categories: &HashMap<&str, usize>,
    new_categories: &'v mut V,
) -> anyhow::Result<()>
where
    V: VecLike<String>,
    &'v mut V: IntoIterator<Item = &'v mut String>,
{
    let mut heap = comparison_categories
        .iter()
        .map(|(c, count)| (*count, *c))
        .collect::<BinaryHeap<_>>();
    println!("the top most common {plural} are: ");
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
        println!("current {plural}:");
        for c in new_categories.iter() {
            println!("\t{c}");
        }
    }
    while let Some(cat) = prompt::prompt(singular).await? {
        new_categories.push(cat);
    }
    for cat in new_categories {
        did_you_mean_check(comparison_categories, cat).await?;
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

async fn category_prompt(
    new_categories: &mut UniqVec<String>,
    mut meta: SongMetadata,
) -> anyhow::Result<SongMetadata> {
    let playlist = Playlist::load().await?;
    let playlist_categories = playlist.categories();
    if let Some(artist) = prompt::prompt_with_default("artist", meta.artist.as_deref()).await? {
        let genres = playlist
            .songs
            .iter()
            .filter(|s| s.artist.as_ref() == Some(&artist))
            .flat_map(|g| g.genres.iter())
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
    list_prompt(LIST_PROMPT_GENRE, &playlist.genres(), &mut meta.genres).await?;
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
    list_prompt(LIST_PROMPT_LIKED_BY, &playlist.likers(), &mut meta.liked_by).await?;
    list_prompt(
        LIST_PROMPT_CATEGORIES,
        &playlist.free_categories(),
        new_categories,
    )
    .await?;
    ensure!(
        !new_categories.is_empty()
            || meta.genres.is_empty()
            || [&meta.artist, &meta.language, &meta.recommended_by]
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

pub(crate) async fn info(song: Vec<String>, just_id: bool, verbose: bool) -> anyhow::Result<()> {
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
            if verbose {
                notify!(
                    "song info:";
                    content:
                        "§bname:§r {}\n§blink:§r {}\n§bartist:§r {}\n§bgenres:§r {}\n§blanguage:§r {}\n§brecommended_by:§r {}\n§bliked_by:§r {}\n§bcategories:§r {}",
                        s.name,
                        s.link,
                        s.artist.iter().format(" | "),
                        s.genres.iter().format(" | "),
                        s.language.iter().format(" | "),
                        s.recommended_by.iter().format(" | "),
                        s.liked_by.iter().format(" | "),
                        s.categories.iter().format(" | "),
                );
            } else {
                notify!(
                    "song info:";
                    content:
                        "§bname:§r {}\n§blink:§r {}\n§bcategories:§r {}",
                        s.name,
                        s.link,
                        s.all_categories().format(" | ")
                );
            }
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

pub async fn edit_categories(
    mut playlist: Playlist,
    song: usize,
    new_categories: UniqVec<String>,
    new_metadata: SongMetadata,
    batch: bool,
) -> anyhow::Result<()> {
    let all_categories = playlist.categories_owned();
    macro_rules! edit {
        ($field:ident) => {
            if let Some(new_f) = new_metadata.$field {
                playlist.songs[song].$field = Some(new_f);
            }
            if !batch {
                let new_f = prompt::prompt_with_default(
                    stringify!($field),
                    playlist.songs[song].$field.as_deref(),
                )
                .await?;
                if let Some(new_f) = new_f {
                    let song = &mut playlist.songs[song];
                    set(
                        &all_categories,
                        &mut song.categories,
                        new_f,
                        &mut song.$field,
                    )
                    .await?;
                } else {
                    playlist.songs[song].$field = None;
                }
            }
        };
    }
    edit!(artist);
    edit!(recommended_by);
    edit!(language);

    macro_rules! edit_list {
        ($prompt:ident, $lister:ident, $field:ident) => {
            let mut list = std::mem::take(&mut playlist.songs[song].$field);
            list.extend(new_metadata.$field);
            if !batch {
                list_prompt($prompt, &playlist.$lister(), &mut list).await?;
                if !list.is_empty() {
                    println!("remove {}", $prompt.1);
                    while let Some(delete) = prompt::interative_select(&list, []).await? {
                        println!("removed {}", &list[delete]);
                        list.remove_at(delete);
                    }
                }
            }
            playlist.songs[song].$field = list;
        };
    }

    edit_list!(LIST_PROMPT_GENRE, genres, genres);
    edit_list!(LIST_PROMPT_LIKED_BY, likers, liked_by);
    let mut categories = std::mem::take(&mut playlist.songs[song].categories);
    categories.extend(new_categories);
    if !batch {
        list_prompt(
            LIST_PROMPT_CATEGORIES,
            &playlist.free_categories(),
            &mut categories,
        )
        .await?;
        if !categories.is_empty() {
            println!("remove {}", LIST_PROMPT_CATEGORIES.1);
            while let Some(delete) = prompt::interative_select(&categories, []).await? {
                println!("removed {}", &categories[delete]);
                categories.remove_at(delete);
            }
        }
    }
    playlist.songs[song].categories = categories;

    notify!("saving edited song as"; content: "{}", playlist.songs[song]);
    playlist.save().await?;
    Ok(())
}
