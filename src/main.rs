mod arg_parse;
mod config;
mod download_ctl;
mod player_ctl;
mod playlist_ctl;
mod queue_ctl;
mod util;

use arg_parse::{Args, Command, DeleteSong, EntityStatus, New};
use clap::{CommandFactory, Parser};
use futures_util::{future::ready, FutureExt, StreamExt, TryFutureExt, TryStreamExt};
use itertools::Itertools;
use mlib::{
    downloaded::{self, clean_downloads},
    item::link::VideoLink,
    players::{self, PlayerIndex, PlayerLink},
    playlist::{PartialSearchResult, Playlist, PlaylistIds},
    queue::Item,
    ytdl::YtdlBuilder,
    Link, Search,
};
use rand::seq::SliceRandom;
use std::{io::Write, process::ExitCode, sync::Mutex};
use tokio::io;
use tracing::dispatcher::set_global_default;
use tracing_log::LogTracer;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};
use util::session_kind::SessionKind;

use crate::{
    arg_parse::{AddPlaylist, Queue},
    config::DownloadFormat,
    util::{dl_dir, selector, with_video::with_video_env},
};

async fn process_cmd(cmd: Command) -> anyhow::Result<()> {
    tracing::debug!(?cmd, "running command");
    match cmd {
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
        Command::Songs { category } => playlist_ctl::songs(category).await?,
        Command::Cat => playlist_ctl::cat().await?,
        Command::Quit => player_ctl::quit().await?,
        Command::Pause => player_ctl::pause().await?,
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
        Command::Loop => player_ctl::toggle_loop().await?,
        Command::New(New {
            search,
            queue,
            query: link,
            categories,
        }) => {
            if categories.is_empty() {
                error!("empty category list"; content: "please provide at least one category");
                return Ok(());
            }
            let link = if search {
                let search = Search::multiple(link, 10);
                notify!("searching for 10 videos....");
                let results = YtdlBuilder::new(&search)
                    .get_title()
                    .search_multiple()?
                    .try_collect::<Vec<_>>()
                    .await?;
                let titles = results.iter().map(|l| l.title_ref()).collect::<Vec<_>>();
                let results_ref = &results;
                match selector::interative_select(
                    &titles,
                    [(
                        'p',
                        Box::new(|_, i| {
                            async move {
                                notify!("loading preview....");
                                if let Err(e) = util::preview_video(results_ref[i].id()).await {
                                    notify!("Error previewing"; content: "{}", e)
                                }
                            }
                            .boxed()
                        }),
                    )],
                )
                .await?
                {
                    Some(pick) => Link::from_video_id(results[pick].id()),
                    None => return Ok(()),
                }
            } else {
                VideoLink::try_from(link)
                    .map_err(|link| anyhow::anyhow!("{} is not a valid link", link))?
                    .into()
            };
            let link = playlist_ctl::new(link, categories).await?;
            if queue {
                queue_ctl::queue(Default::default(), Some(Item::Link(link.into()))).await?;
            }
        }
        Command::AddPlaylist(AddPlaylist {
            queue,
            link,
            categories,
        }) => {
            let link =
                Link::try_from(link).map_err(|s| anyhow::anyhow!("{} is not a valid link", s))?;
            let links = playlist_ctl::add_playlist(&link, categories).await?;
            if queue {
                links
                    .for_each(|r| async move {
                        let r = ready(r)
                            .and_then(|link| {
                                queue_ctl::queue(Default::default(), Some(Item::Link(link.into())))
                            })
                            .await;
                        if let Err(e) = r {
                            tracing::error!("failed adding item to playlist: {:?}", e)
                        }
                    })
                    .await;
            } else {
                links.for_each(|_| ready(())).await;
            }
        }
        Command::Current { link, notify } => queue_ctl::current(link, notify).await?,
        Command::Now(a) => queue_ctl::now(a).await?,
        Command::CleanDownloads => {
            let ids = PlaylistIds::load().await?;
            let to_delete = clean_downloads(dl_dir().await?, &ids).await?;
            tokio::pin!(to_delete);
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
        Command::Dump { file } => queue_ctl::dump(file).await?,
        Command::Load { file, shuf } => queue_ctl::load(file, shuf).await?,
        Command::Play(arg_parse::Play {
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
        Command::ChCat => playlist_ctl::ch_cat().await?,
        Command::DeleteSong(DeleteSong {
            current,
            partial_name,
        }) => playlist_ctl::delete_song(current, partial_name).await?,
        Command::Queue(Queue {
            queue_opts,
            play_opts,
        }) => {
            let items =
                search_params_to_items(play_opts.what, play_opts.search, play_opts.category)
                    .await?;
            queue_ctl::queue(queue_opts, items).await?;
        }
        Command::Dequeue(d) => queue_ctl::dequeue(d).await?,
        Command::Playlist => queue_ctl::run_interactive_playlist().await?,
        Command::Status { entity } => match entity {
            EntityStatus::Players => player_ctl::status().await?,
            EntityStatus::Cache => download_ctl::cache_status().await?,
            EntityStatus::Downloads => download_ctl::daemon_status().await?,
        },
        Command::Interactive => player_ctl::interactive().await?,
        Command::Lyrics => {
            dbg!(
                selector::interative_select(
                    &["option 1", "option 2"],
                    [(
                        'p',
                        Box::new(|e, _| {
                            async move { notify!("{}", e; force_notify: true) }.boxed()
                        })
                    )]
                )
                .await
            )?;
        }
        Command::Info { song } => playlist_ctl::info(song).await?,
        Command::AutoComplete { shell } => {
            clap_complete::generate(
                shell,
                &mut Args::command(),
                "m",
                &mut std::io::stdout().lock(),
            );
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
                                if let Err(e) = downloaded::download(
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
                        Link::Playlist(_) => {}
                        Link::OtherPlatform(_) => {}
                    },
                    Item::File(_) => {}
                    Item::Search(_) => {}
                }
            }
        }
    }
    tracing::debug!("updating bar");
    // TODO: move this somewhere that only runs when actual updates happen
    util::update_bar().await?;

    Ok(())
}

struct TracedWriter<W: Write>(W);

impl<W> Write for TracedWriter<W>
where
    W: Write,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        log_if_err(self.0.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        log_if_err(self.0.flush())
    }

    fn write_vectored(&mut self, bufs: &[std::io::IoSlice<'_>]) -> io::Result<usize> {
        log_if_err(self.0.write_vectored(bufs))
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        log_if_err(self.0.write_all(buf))
    }

    fn write_fmt(&mut self, fmt: std::fmt::Arguments<'_>) -> io::Result<()> {
        log_if_err(self.0.write_fmt(fmt))
    }
}

fn log_if_err<T, E: std::fmt::Debug>(r: Result<T, E>) -> Result<T, E> {
    if let Err(e) = &r {
        tracing::error!("{:?}", e)
    }
    r
}

static CHOSEN_INDEX: Mutex<PlayerIndex> = Mutex::new(PlayerIndex::CURRENT);
pub fn chosen_index() -> PlayerLink {
    PlayerLink::from(*CHOSEN_INDEX.lock().unwrap())
}

async fn run() -> anyhow::Result<()> {
    download_ctl::start_daemon_if_running_as_daemon().await?;
    players::start_daemon_if_running_as_daemon().await?;

    let args = match Args::try_parse() {
        Ok(args) => args,
        Err(e) => {
            if let SessionKind::Gui = SessionKind::current().await {
                error!("Invalid arguments"; content: "{:?}", e)
            }
            e.exit()
        }
    };
    if let Some(id) = args.socket {
        *CHOSEN_INDEX.lock().unwrap() = PlayerIndex::of(id);
    }

    if let Some(new_base) = config::CONFIG.socket_base_dir.as_ref() {
        players::override_legacy_socket_base_dir(new_base.clone());
    }

    if let Some(cmd) = args.cmd {
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
        let mut items = vec![];
        let mut words = vec![];
        for x in strings {
            match Link::try_from(x) {
                Ok(l) => items.push(Item::Link(l)),
                Err(s) => match tokio::fs::metadata(&s).await {
                    Ok(_) => items.push(Item::File(s.into())),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => words.push(s),
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
    let SongQuery { mut items, words } = {
        tracing::debug!(?what, "parsing query");
        let mut query = SongQuery::new(what).await;
        if let Some(cat) = category {
            let cat = &cat;
            let cat_items = Playlist::stream()
                .await?
                .filter_map(|s| async { s.ok() })
                .filter_map(|s| async move {
                    s.categories
                        .iter()
                        .any(|c| c.contains(cat))
                        .then_some(s.link)
                })
                .map(Link::Video)
                .map(Item::Link)
                .collect::<Vec<_>>()
                .await;
            query.items.extend(cat_items);
            query.items.shuffle(&mut rand::rngs::OsRng);
        }
        tracing::debug!(?query, "created song query");
        query
    };
    let link = if words.is_empty() {
        return Ok(items);
    } else if search {
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
    Ok(items)
}
