mod arg_parse;
mod player_ctl;
mod playlist_ctl;
mod queue_ctl;
mod util;

use arg_parse::{Command, DeleteSong, New, Play};
use futures_util::{future::ready, StreamExt, TryFutureExt};
use mlib::{
    downloaded::clean_downloads,
    playlist::{Playlist, PlaylistIds},
    queue::Item,
    socket::MpvSocket,
    Error as SockErr, Link, Search,
};
use rand::seq::SliceRandom;
use structopt::StructOpt;
use tokio::io;
use tracing::dispatcher::set_global_default;
use tracing_log::LogTracer;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

async fn run() -> anyhow::Result<()> {
    let cmd = Command::from_args();
    match cmd {
        Command::Socket { new } => {
            if new.is_some() {
                println!("{}", MpvSocket::new_path().await?.display());
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
            queue,
            link,
            categories,
        }) => {
            let link = playlist_ctl::new(link, categories).await?;
            if queue {
                queue_ctl::queue(Default::default(), Some(Item::Link(link))).await?;
            }
        }
        Command::AddPlaylist(New {
            queue,
            link,
            categories,
        }) => {
            let links = playlist_ctl::add_playlist(link, categories).await?;
            if queue {
                links
                    .for_each(|r| async move {
                        let r = ready(r)
                            .and_then(|link| {
                                queue_ctl::queue(Default::default(), Some(Item::Link(link)))
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
            let to_delete = clean_downloads(&ids).await?;
            tokio::pin!(to_delete);
            while let Some(f) = to_delete.next().await {
                match f {
                    Ok(f) => {
                        if let Err(e) = tokio::fs::remove_file(&f).await {
                            error!("Failed to delete {}", f.display(); content: "{}", e)
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
                    .map(Item::Link)
                    .collect::<Vec<_>>()
                    .await;
                items.extend(cat_items);
                items.shuffle(&mut rand::rngs::OsRng);
            }
            queue_ctl::queue(opts, items).await?;
        }
        Command::Dequeue(d) => queue_ctl::dequeue(d).await?,
        _ => todo!()
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

    let fmt = fmt::layer().event_format(fmt::format());

    let sub = Registry::default().with(env_filter).with(fmt);

    set_global_default(sub.into()).expect("Failed to set global default");
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logger();
    if let Err(e) = run().await {
        error!("{:?}", e);
    }
    Ok(())
}

fn handle_search_result<T>(r: Result<Option<T>, usize>) -> anyhow::Result<T> {
    match r {
        Ok(Some(t)) => Ok(t),
        Ok(None) => return Err(anyhow::anyhow!("song not in playlist")),
        Err(too_many_matches) => {
            return Err(anyhow::anyhow!("too many matches: {}", too_many_matches))
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
    let SongQuery { mut items, words } = SongQuery::new(what).await;
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
            .link,
        )
    };
    items.push(link);
    Ok(items)
}
