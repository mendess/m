mod arg_parse;
mod config;
mod download_ctl;
mod events;
mod player_ctl;
mod playlist_ctl;
mod queue_ctl;
mod util;

use arg_parse::{Args, Command, DeleteSong, EntityStatus, New, Play};
use clap::{CommandFactory, Parser};
use futures_util::StreamExt;
use itertools::Itertools;
use mlib::{
    Link, Search,
    downloaded::{self, clean_downloads},
    item::{
        self,
        link::{BangerLink, HasId},
    },
    players::{self, PlayerIndex, PlayerLink},
    playlist::{PartialSearchResult, Playlist, PlaylistIds},
    queue::Item,
};
use rand::seq::SliceRandom;
use std::{pin::pin, process::ExitCode, sync::Mutex};
use tokio::io;
use tracing::dispatcher::set_global_default;
use tracing_log::LogTracer;
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt};
use util::session_kind::SessionKind;

use crate::{
    arg_parse::Queue,
    config::DownloadFormat,
    util::{dl_dir, with_video::with_video_env},
};
use anyhow::Context;

#[tracing::instrument]
async fn process_cmd(cmd: Command) -> anyhow::Result<()> {
    tracing::debug!(?cmd, "running command");
    match cmd {
        Command::Songs { category } => playlist_ctl::songs(category).await?,
        Command::Cat { kind } => playlist_ctl::ls_categories(kind).await?,
        Command::Quit => player_ctl::quit().await?,
        Command::SetPlay => player_ctl::resume().await?,
        Command::SetPause => player_ctl::pause().await?,
        Command::Pause => player_ctl::cycle_pause().await?,
        Command::Vu(a) => player_ctl::vu(a).await?,
        Command::Vd(a) => player_ctl::vd(a).await?,
        Command::ToggleVideo => player_ctl::toggle_video().await?,
        Command::NextFile(a) => player_ctl::next_file(a).await?,
        Command::PrevFile(a) => player_ctl::prev_file(a).await?,
        Command::Frwd(a) => player_ctl::frwd(a).await?,
        Command::Back(a) => player_ctl::back(a).await?,
        Command::Next(a) => player_ctl::next(a).await?,
        Command::Prev(a) => player_ctl::prev(a).await?,
        Command::Shuffle => player_ctl::shuffle().await?,
        Command::Loop { flag: None } => player_ctl::toggle_loop().await?,
        Command::Loop { flag: Some(state) } => player_ctl::set_looping(state).await?,
        Command::Now(a) => queue_ctl::now(a).await?,
        Command::Dump { file } => queue_ctl::dump(file).await?,
        Command::Load { file, shuf } => queue_ctl::load(file, shuf).await?,
        Command::DeleteSong(DeleteSong {
            current,
            partial_name,
        }) => playlist_ctl::delete_song(current, partial_name).await?,
        Command::Dequeue(d) => queue_ctl::dequeue(d).await?,
        Command::Playlist => queue_ctl::run_interactive_playlist().await?,
        Command::Status { entity } => match entity {
            Some(EntityStatus::Players) => player_ctl::status().await?,
            Some(EntityStatus::Cache) => download_ctl::cache_status().await?,
            Some(EntityStatus::Downloads) => download_ctl::daemon_status().await?,
            None => {
                player_ctl::status().await?;
                download_ctl::cache_status().await?;
                download_ctl::daemon_status().await?;
            }
        },
        Command::Interactive => player_ctl::interactive().await?,
        Command::Lyrics => todo!("lyrics not implemented"),
        Command::Info { id, song, verbose } => playlist_ctl::info(song, id, verbose).await?,
        Command::Socket { new } => {
            if new.is_some() {
                println!(
                    "{}",
                    players::legacy_socket_for(players::current().await?.unwrap_or_default() + 1)
                        .await
                );
            } else {
                match players::current().await? {
                    Some(i) => println!("{}", players::legacy_socket_for(i).await),
                    None => println!("/dev/null"),
                }
            }
        }
        Command::Play(Play {
            search,
            what,
            category,
            video,
        }) => {
            queue_ctl::play(
                search_params_to_items(what, search, category).await?,
                video || with_video_env(),
            )
            .await?;
        }
        Command::Queue(Queue {
            queue_opts,
            play_opts,
        }) => {
            let items =
                search_params_to_items(play_opts.what, play_opts.search, play_opts.category)
                    .await?;
            queue_ctl::queue(queue_opts, items).await?;
        }
        Command::AutoComplete { shell } => {
            clap_complete::generate(
                shell,
                &mut Args::command(),
                "m",
                &mut std::io::stdout().lock(),
            );
        }
        Command::New(New {
            queue,
            query,
            metadata,
            categories,
            batch,
        }) => {
            let links = query
                .into_iter()
                .map(BangerLink::try_from)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|link| anyhow::anyhow!("{} is not a valid link", link))?;
            playlist_ctl::new(&links, metadata, categories.into(), batch).await?;
            if queue {
                queue_ctl::queue(
                    Default::default(),
                    links.into_iter().map(|i| Item::from(item::Link::from(i))),
                )
                .await?;
            }
        }
        Command::Current {
            link,
            short,
            notify,
        } => {
            queue_ctl::current(
                match link {
                    0 => queue_ctl::CurrentDisplayMode::Default { short },
                    1 => queue_ctl::CurrentDisplayMode::Link,
                    2.. => queue_ctl::CurrentDisplayMode::LinkId,
                },
                notify,
            )
            .await?
        }
        Command::CleanDownloads => {
            let ids = PlaylistIds::load().await?;
            let mut to_delete = pin!(clean_downloads(dl_dir().await?, &ids).await?);
            while let Some(f) = to_delete.next().await {
                match f {
                    Ok(f) => {
                        if let Err(e) = tokio::fs::remove_file(&f).await {
                            error!("Failed to delete {}", f.display(); content: "{}", e)
                        } else {
                            notify!("deleted {}", f.display());
                        }
                    }
                    Err(e) => {
                        error!("something went wrong when inspecting a file"; content: "{}", e)
                    }
                }
            }
        }
        Command::Download { what, category } => {
            let items = if what.is_none() && category.is_none() {
                Playlist::load()
                    .await?
                    .songs
                    .into_iter()
                    .map(|i| Item::Link(i.link.into()))
                    .collect()
            } else {
                search_params_to_items(what.unwrap_or_default(), false, category).await?
            };
            let dl_dir = dl_dir().await?;
            let total = items.len();
            for (idx, i) in items.into_iter().enumerate() {
                match i {
                    Item::Link(l) => match l {
                        Link::Video(l) => {
                            if !downloaded::is_in_cache(&dl_dir, &l).await {
                                notify!("[{idx}/{total}] downloading {l}");
                                if let Err(e) = downloaded::yt_download(
                                    dl_dir.clone(),
                                    &l,
                                    config::CONFIG.download_format == DownloadFormat::Audio,
                                )
                                .await
                                {
                                    tracing::error!(?e, "failed to download {l}");
                                }
                            }
                        }
                        Link::Banger(l) => {
                            if !downloaded::is_in_cache(&dl_dir, &l).await {
                                notify!("[{idx}/{total}] downloading {l}");
                                if let Err(e) = downloaded::download(dl_dir.clone(), &l).await {
                                    tracing::error!(?e, "failed to download {l}");
                                }
                            }
                        }
                        Link::Playlist(link) => {
                            tracing::warn!(?link, "donwloading playlists is not supported")
                        }
                        Link::Channel(link) => {
                            tracing::warn!(?link, "donwloading channels is not supported")
                        }
                        Link::OtherPlatform(link) => {
                            tracing::warn!(
                                ?link,
                                "donwloading from other platforms is not supported"
                            )
                        }
                    },
                    Item::File(_) => {}
                    Item::Search(_) => {}
                }
            }
        }
        Command::ChangeCategories {
            current,
            song,
            categories,
            metadata,
            batch,
        } => {
            let playlist = Playlist::load().await?;
            let song = match song {
                _ if current => {
                    let filename = PlayerLink::current().playing_item().await?;
                    let Item::Link(item::link::Link::Banger(link)) = filename else {
                        anyhow::bail!("not currently playing a playlist item");
                    };
                    playlist
                        .song_index_by_link(&link)
                        .context("song not in playlist")?
                }
                Some(song) => match BangerLink::try_from(song) {
                    Ok(link) => playlist
                        .song_index_by_id(link.id())
                        .context("link not in playlist")?,
                    Err(song) => crate::handle_search_result(
                        playlist.partial_name_search(song.split_whitespace()),
                    )?
                    .into_index(),
                },
                None => anyhow::bail!("please provide a song to edit"),
            };
            notify!("editing song"; content: "{}", playlist.songs[song].name);
            playlist_ctl::edit_categories(playlist, song, categories.into(), metadata, batch)
                .await?
        }
        Command::Events => events::display().await?,
    }

    Ok(())
}

static CHOSEN_INDEX: Mutex<PlayerIndex> = Mutex::new(PlayerIndex::CURRENT);
pub fn chosen_index() -> PlayerLink {
    PlayerLink::from(*CHOSEN_INDEX.lock().unwrap())
}

async fn run() -> anyhow::Result<()> {
    download_ctl::start_daemon_if_running_as_daemon().await?;
    players::start_daemon_if_running_as_daemon(Default::default(), Default::default()).await?;

    let Args {
        socket,
        cookies_browser,
        cookies_path,
        cmd,
    } = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            if let SessionKind::Gui = SessionKind::current().await {
                anyhow::bail!("{e}")
            } else {
                e.exit()
            }
        }
    };
    if let Some(id) = socket {
        *CHOSEN_INDEX.lock().unwrap() = PlayerIndex::of(id);
    }

    if let Some(new_base) = config::CONFIG.socket_base_dir.as_ref() {
        players::override_legacy_socket_base_dir(new_base.clone());
    }

    if let Some(cookies_path) = cookies_path {
        mlib::ytdl::set_cookies_path(&cookies_path);
    }

    if let Some(cookies_browser) = cookies_browser {
        mlib::ytdl::set_cookies_browser(&cookies_browser).await?;
    }

    if let Some(cmd) = cmd {
        process_cmd(cmd).await?;
    } else {
        player_ctl::interactive().await?;
    }

    Ok(())
}

pub fn init_logger() {
    LogTracer::init().expect("Failed to set logger");

    let env_filter = if let Ok(e) = EnvFilter::try_from_default_env() {
        e
    } else {
        EnvFilter::new("warn")
    };

    let fmt = fmt::layer().with_writer(std::io::stderr).pretty();

    let sub = Registry::default().with(env_filter).with(fmt);

    set_global_default(sub.into()).expect("Failed to set global default");
}

#[tokio::main]
async fn main() -> ExitCode {
    init_logger();
    if let Err(e) = run().await {
        let mut chain = e.chain().skip(1).peekable();
        let stringified = e.to_string();
        let (header, rest) = match stringified.split_once('\n') {
            Some(x) => x,
            None => (stringified.as_str(), ""),
        };
        if chain.peek().is_some() {
            error!("{}", header; content: "{}Caused by:\n\t{}", rest, chain.format("\n\t"));
        } else {
            error!("{}", header; content: "{}", rest);
        }
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn handle_search_result<T>(r: PartialSearchResult<T>) -> anyhow::Result<T> {
    match r {
        PartialSearchResult::One(t) => Ok(t),
        PartialSearchResult::None => Err(anyhow::anyhow!("song not in playlist")),
        PartialSearchResult::Many(too_many_matches) => Err(anyhow::anyhow!(
            "too many matches:\n  {}",
            too_many_matches.into_iter().format("\n  ")
        )),
    }
}

#[derive(Debug)]
pub struct SongQuery {
    pub items: Vec<Item>,
    pub words: Vec<String>,
}

impl SongQuery {
    pub async fn new(strings: Vec<String>) -> Self {
        tracing::debug!(?strings, "parsing song query");
        let mut items = vec![];
        let mut words = vec![];
        for x in strings {
            match Link::try_from(x) {
                Ok(l) => {
                    tracing::debug!(link = ?l, "found link");
                    items.push(Item::Link(l))
                }
                Err(s) => match tokio::fs::metadata(&s).await {
                    Ok(_) => {
                        tracing::debug!(file = ?s, "found file");
                        let path = std::env::current_dir()
                            .map(|pwd| pwd.join(&s))
                            .unwrap_or_else(|_| s.into());
                        items.push(Item::File(path))
                    }
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        tracing::debug!(word = ?s, "using as search word");
                        words.push(s)
                    }
                    Err(e) => {
                        tracing::error!("error checking if {:?} was a path to a file: {:?}", s, e)
                    }
                },
            }
        }
        Self { items, words }
    }
}

async fn search_params_to_items(
    what: Vec<String>,
    search: bool,
    category: Option<String>,
) -> anyhow::Result<Vec<Item>> {
    tracing::debug!(?what, "parsing query");

    let SongQuery { mut items, words } = SongQuery::new(what).await;

    if let Some(cat) = category {
        let cat = &cat;
        let cat_items = Playlist::stream()
            .await?
            .filter_map(|s| async { s.ok() })
            .filter_map(|s| async move {
                let contains = s.all_categories().any(|c| c.contains(cat));
                contains.then_some(s.link)
            })
            .map(Link::from)
            .map(Item::Link)
            .collect::<Vec<_>>()
            .await;
        items.extend(cat_items);
        items.shuffle(&mut rand::rngs::OsRng);
    }

    if !words.is_empty() {
        let link = if search {
            Item::Search(Search::new(words.join(" ")))
        } else {
            Item::Link(
                handle_search_result(
                    Playlist::load()
                        .await?
                        .partial_name_search_mut(words.iter().map(String::as_str)),
                )?
                .delete()
                .link
                .into(),
            )
        };
        items.push(link);
    }
    Ok(items)
}
