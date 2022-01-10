use std::{
    io::IoSlice, os::unix::process::CommandExt, path::PathBuf, process::Command, time::Duration,
};

use anyhow::Context;
use futures_util::{stream::FuturesUnordered, StreamExt};
use itertools::Itertools;
use mlib::{
    downloaded,
    downloaded::{check_cache, CheckCacheDecision},
    item::link::VideoLink,
    playlist::Playlist,
    Item,
};
use once_cell::sync::Lazy;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{UnixListener, UnixStream},
    sync::{
        mpsc::{self, Sender},
        OnceCell,
    },
    time::timeout,
};

use crate::error;

fn socket_path() -> PathBuf {
    static PATH: Lazy<PathBuf> = Lazy::new(|| {
        PathBuf::from_iter([
            String::from("/tmp"),
            whoami::username(),
            String::from(ARG_0),
        ])
    });
    PATH.clone()
}

async fn handle_task(stream: UnixStream, tx: Sender<VideoLink>) {
    let mut lines = BufReader::new(stream).lines();
    loop {
        match lines
            .next_line()
            .await
            .map(|opt| opt.map(VideoLink::from_url))
        {
            Ok(Some(Ok(link))) => {
                if tx.send(link).await.is_err() {
                    break;
                }
            }
            Ok(Some(Err(s))) => {
                tracing::warn!("received a non link: {:?}", s);
            }
            Ok(None) => break,
            Err(e) => {
                tracing::warn!("error reading line: {:?}", e);
            }
        }
    }
}

pub async fn download_daemon() -> anyhow::Result<()> {
    let socket_path = socket_path();
    let _ = tokio::fs::remove_file(&socket_path).await;
    let socket = UnixListener::bind(&socket_path)
        .with_context(|| format!("path: {}", socket_path.display()))?;

    let (tx, mut rx) = mpsc::channel(100);
    let dl_dir = crate::util::dl_dir()?;

    tokio::spawn(async move {
        loop {
            match socket.accept().await {
                Ok((stream, addr)) => drop(tokio::spawn({
                    tracing::info!("got a new connection from {:?}", addr);
                    let tx = tx.clone();
                    handle_task(stream, tx)
                })),
                Err(e) => {
                    tracing::error!("failed to accept connection: {:?}", e);
                }
            }
        }
    });

    let mut task_set = FuturesUnordered::new();

    while let Ok(Some(l)) = timeout(Duration::from_secs(60), rx.recv()).await {
        task_set.push(tokio::spawn(downloaded::download(dl_dir.clone(), l)));

        while task_set.len() > 8 {
            match task_set.next().await.unwrap() {
                Ok(Ok(l)) => tracing::info!("downloaded {}", l),
                Ok(Err(e)) => error!("error downloading link: {:?}", e),
                Err(e) => error!("error joining download task: {:?}", e),
            }
        }
    }
    task_set
        .for_each(|l| async {
            match l {
                Ok(Ok(l)) => tracing::info!("downloaded {}", l),
                Ok(Err(e)) => error!("error downloading link: {:?}", e),
                Err(e) => error!("error joining download task: {:?}", e),
            }
        })
        .await;
    let _ = tokio::fs::remove_file(&socket_path).await;
    Ok(())
}

pub const ARG_0: &str = "into-the-m-verse";

static DL_SERVER_SINK: OnceCell<Sender<VideoLink>> = OnceCell::const_new();

async fn create_server_sink() -> anyhow::Result<Sender<VideoLink>> {
    fn forward(mut sock: UnixStream) -> Sender<VideoLink> {
        let (tx, mut rx) = mpsc::channel::<VideoLink>(100);
        tokio::spawn(async move {
            while let Some(l) = rx.recv().await {
                let vector = [IoSlice::new(l.as_str().as_bytes()), IoSlice::new(b"\n")];
                if let Err(e) = sock.write_vectored(&vector).await {
                    tracing::warn!("failed to talk to download server: {:?}", e)
                }
            }
        });

        tx
    }

    let socket_path = socket_path();

    // try to connect to an existing server
    if let Ok(tx) = UnixStream::connect(&socket_path).await.map(forward) {
        return Ok(tx);
    }

    // start the download daemon
    Command::new("/proc/self/exe").arg0(ARG_0).spawn()?;

    // wait for and try to connect to the new daemon
    for i in 1..=5 {
        if let Ok(tx) = UnixStream::connect(&socket_path).await.map(forward) {
            return Ok(tx);
        } else {
            tokio::time::sleep(Duration::from_millis(100 * i)).await;
        }
    }
    Err(anyhow::anyhow!("could not connect to server"))
}

pub async fn check_cache_ref(path: PathBuf, item: &mut Item) {
    match mlib::downloaded::check_cache_ref(path, item).await {
        CheckCacheDecision::Skip => {}
        CheckCacheDecision::Download(l) => {
            match DL_SERVER_SINK.get_or_try_init(create_server_sink).await {
                Ok(tx) => {
                    if tx.send(l).await.is_err() {
                        // TODO: clear server
                    }
                }
                Err(e) => {
                    crate::error!("failed to start myself: {:?}", e);
                }
            }
        }
    }
}

pub async fn cache_status() -> anyhow::Result<()> {
    let dl_dir = crate::dl_dir()?;
    let dl_dir = &dl_dir;
    let (cached, not) = Playlist::stream()
        .await?
        .filter_map(|r| async { r.ok() })
        .fold((vec![], vec![]), |(mut cached, mut not), s| async move {
            if check_cache(dl_dir, &s.link).await {
                cached.push(s.name);
            } else {
                not.push(s.name);
            }

            (cached, not)
        })
        .await;
    crate::notify!("Cache status";
        content:
            "   Cached: {}\nNot Cached: {}\nMissing:\n  {}",
            cached.len(),
            not.len(),
            not.iter().format("\n  ")
    );
    Ok(())
}
