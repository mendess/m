mod arg_parse;
mod download_ctl;
mod player_ctl;
mod playlist_ctl;
mod queue_ctl;
mod util;

use arg_parse::{Args, Command, DeleteSong, EntityStatus, New, Play};
use futures_util::{future::ready, FutureExt, StreamExt, TryFutureExt, TryStreamExt};
use itertools::Itertools;
use mlib::{
    downloaded::clean_downloads,
    item::link::VideoLink,
    playlist::{PartialSearchResult, Playlist, PlaylistIds},
    queue::Item,
    socket::MpvSocket,
    ytdl::YtdlBuilder,
    Error as SockErr, Link, Search,
};
use rand::seq::SliceRandom;
use std::{env::args, io::Write};
use structopt::StructOpt;
use tokio::io;
use tracing::dispatcher::set_global_default;
use tracing_log::LogTracer;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};
use util::session_kind::SessionKind;

use crate::{
    arg_parse::AddPlaylist,
    util::{dl_dir, selector},
};

use async_recursion::async_recursion;

#[async_recursion(?Send)]
async fn process_cmd(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        Command::Socket { new } => {
            if new.is_some() {
                println!("{}", MpvSocket::new_unconnected().await?.path().display());
            } else {
                match MpvSocket::lattest().await {
                    Ok(s) => println!("{}", s.path().display()),
                    Err(SockErr::NoMpvInstance) => println!("/dev/null"),
                    Err(e) => return Err(e.into()),
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
                VideoLink::from_url(link)
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
                Link::from_url(link).map_err(|s| anyhow::anyhow!("{} is not a valid link", s))?;
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
            let to_delete = clean_downloads(dl_dir()?, &ids).await?;
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
        Command::Load { file } => queue_ctl::load(file).await?,
        Command::Play(p) => queue_ctl::play(search_params_to_items(p).await?, false).await?,
        Command::ChCat => playlist_ctl::ch_cat().await?,
        Command::DeleteSong(DeleteSong {
            current,
            partial_name,
        }) => playlist_ctl::delete_song(current, partial_name).await?,
        Command::Queue(q) => {
            let mut opts = q.queue_opts;
            let mut items = search_params_to_items(q.play_opts).await?;
            if let Some(cat) = opts.category.take() {
                let cat = &cat;
                let cat_items = Playlist::stream()
                    .await?
                    .filter_map(|s| async { s.ok() })
                    .filter_map(|s| async move {
                        s.categories.iter().any(|c| c.contains(cat)).then(|| s.link)
                    })
                    .map(Link::Video)
                    .map(Item::Link)
                    .collect::<Vec<_>>()
                    .await;
                items.extend(cat_items);
                items.shuffle(&mut rand::rngs::OsRng);
            }
            queue_ctl::queue(opts, items).await?;
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
            Args::clap().gen_completions_to("m", shell, &mut TracedWriter(std::io::stdout().lock()))
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

async fn run() -> anyhow::Result<()> {
    if args().next().as_deref() == Some(download_ctl::ARG_0) {
        return download_ctl::download_daemon().await;
    }
    let args = match Args::from_args_safe() {
        Ok(args) => args,
        Err(e) => {
            if let SessionKind::Gui = SessionKind::current().await {
                error!("Invalid arguments"; content: "{:?}", e)
            }
            e.exit()
        }
    };
    if let Some(id) = args.socket {
        mlib::socket::override_lattest(id);
    }

    process_cmd(args.cmd).await?;

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
async fn main() {
    init_logger();
    let exit_code = {
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
            1
        } else {
            0
        }
    };
    std::process::exit(exit_code);
}

fn handle_search_result<T>(r: PartialSearchResult<T>) -> anyhow::Result<T> {
    match r {
        PartialSearchResult::One(t) => Ok(t),
        PartialSearchResult::None => return Err(anyhow::anyhow!("song not in playlist")),
        PartialSearchResult::Many(too_many_matches) => {
            return Err(anyhow::anyhow!(
                "too many matches:\n  {}",
                too_many_matches.into_iter().format("\n  ")
            ))
        }
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
            match Link::from_url(x) {
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

async fn search_params_to_items(Play { what, search }: Play) -> anyhow::Result<Vec<Item>> {
    let SongQuery { mut items, words } = {
        tracing::debug!(?what, "parsing query");
        let query = SongQuery::new(what).await;
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
