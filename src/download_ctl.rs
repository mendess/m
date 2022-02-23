use std::{
    io::IoSlice, os::unix::process::CommandExt, path::PathBuf, process::Command, time::Duration,
};

use anyhow::Context;
use futures_util::StreamExt;
use itertools::Itertools;
use mlib::{
    downloaded::{check_cache, CheckCacheDecision},
    playlist::Playlist,
    Item,
};
use once_cell::sync::Lazy;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::UnixStream,
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex, OnceCell,
    },
};

fn socket_path() -> PathBuf {
    static PATH: Lazy<PathBuf> = Lazy::new(|| {
        let (path, e) = namespaced_tmp::blocking::in_user_tmp(ARG_0);
        tracing::error!("failed to create tmp dir for download daemon: {:?}", e);
        path
    });
    PATH.clone()
}

mod daemon {
    use std::{
        iter::once,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    use anyhow::Context;
    use futures_util::{stream::FuturesUnordered, StreamExt};
    use mlib::{downloaded, item::link::VideoLink};
    use once_cell::sync::Lazy;
    use tokio::{
        io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
        net::{UnixListener, UnixStream},
        sync::{
            mpsc::{self, Sender},
            Mutex,
        },
        time::timeout,
    };

    use crate::error;

    use super::socket_path;

    #[derive(Debug, Default)]
    struct Status {
        done: Vec<VideoLink>,
        downloading: Vec<VideoLink>,
    }

    static STATUS: Lazy<Mutex<Status>> = Lazy::new(Mutex::default);
    static QUEUED_COUNT: AtomicUsize = AtomicUsize::new(0);
    pub(super) const SEPERATOR: &str = "========";
    pub(super) const TERMINATOR: &str = "<<<<<<<<";

    async fn handle_task(mut stream: UnixStream, tx: Sender<VideoLink>) {
        let (recv, send) = stream.split();
        let mut lines = BufReader::new(recv).lines();
        let mut send = BufWriter::new(send);
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
                    QUEUED_COUNT.fetch_add(1, Ordering::Relaxed);
                }
                Ok(Some(Err(s))) if s == "status" => {
                    tracing::info!("got status request");
                    let status = STATUS.lock().await;
                    tracing::debug!("current status {:?}", status);
                    let queued_count = QUEUED_COUNT.load(Ordering::Relaxed).to_string();
                    let mut status_lines = futures_util::stream::iter(
                        once(queued_count.as_str())
                            .chain(status.done.iter().map(|l| l.as_str()))
                            .chain(once(SEPERATOR))
                            .chain(status.downloading.iter().map(|l| l.as_str()))
                            .chain(once(TERMINATOR)),
                    );
                    while let Some(l) = status_lines.next().await {
                        let r = async {
                            send.write_all(l.as_bytes()).await?;
                            send.write_all(b"\n").await?;
                            send.flush().await?;
                            tracing::debug!("sent {}", l);
                            Result::<_, std::io::Error>::Ok(())
                        }
                        .await;
                        if let Err(e) = r {
                            tracing::error!("failed to send status: {:?}", e);
                        }
                    }
                    tracing::debug!("status request fulfilled");
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
            QUEUED_COUNT.fetch_sub(1, Ordering::Relaxed);
            STATUS.lock().await.downloading.push(l.clone());
            task_set.push(tokio::spawn(downloaded::download(dl_dir.clone(), l)));

            while task_set.len() >= 8 {
                match task_set.next().await.unwrap() {
                    Ok(Ok(l)) => {
                        tracing::info!("downloaded {}", l);
                        let mut status = STATUS.lock().await;
                        status.downloading.retain(|e| e != &l);
                        status.done.push(l);
                    }
                    Ok(Err(e)) => error!("error downloading link: {:?}", e),
                    Err(e) => error!("error joining download task: {:?}", e),
                }
            }
        }
        task_set
            .for_each(|l| async {
                match l {
                    Ok(Ok(l)) => {
                        tracing::info!("downloaded {}", l);
                        let mut status = STATUS.lock().await;
                        status.downloading.retain(|e| e != &l);
                        status.done.push(l);
                    }
                    Ok(Err(e)) => error!("error downloading link: {:?}", e),
                    Err(e) => error!("error joining download task: {:?}", e),
                }
            })
            .await;
        let _ = tokio::fs::remove_file(&socket_path).await;
        Ok(())
    }
}

pub use daemon::download_daemon;

pub const ARG_0: &str = "into-the-m-verse";

struct Channels {
    sender: Sender<String>,
    receiver: Mutex<Receiver<String>>,
}

static DL_SERVER_SINK: OnceCell<Channels> = OnceCell::const_new();

async fn create_server_sink() -> anyhow::Result<Channels> {
    fn forward(sock: UnixStream) -> Channels {
        let (tx0, mut rx0) = mpsc::channel::<String>(100);
        let (tx1, rx1) = mpsc::channel::<String>(100);
        let (recv, mut send) = sock.into_split();
        tokio::spawn(async move {
            while let Some(l) = rx0.recv().await {
                let vector = [IoSlice::new(l.as_bytes()), IoSlice::new(b"\n")];
                if let Err(e) = send.write_vectored(&vector).await {
                    tracing::warn!("failed to talk to download server: {:?}", e)
                }
            }
        });
        tokio::spawn(async move {
            let mut lines = BufReader::new(recv).lines();
            tracing::debug!("started read from daemon task");
            while let Ok(Some(l)) = lines.next_line().await {
                tracing::debug!("read {} from daemon", l);
                if let Err(e) = tx1.send(l).await {
                    tracing::warn!("failed to report server msg: {:?}", e)
                }
            }
            tracing::warn!("read from daemon task terminating");
        });

        Channels {
            sender: tx0,
            receiver: Mutex::new(rx1),
        }
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
                    if tx.sender.send(l.into_string()).await.is_err() {
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

pub async fn daemon_status() -> anyhow::Result<()> {
    let sinks = DL_SERVER_SINK.get_or_try_init(create_server_sink).await?;
    sinks.sender.send("status".into()).await?;
    tracing::debug!("sent status request");
    let mut rx = sinks.receiver.lock().await;
    tracing::debug!("acquired receiver lock");
    let queued_count = rx
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("failed to get queued count"))?
        .parse::<usize>()
        .context("failed to parse queued count")?;
    let mut done = vec![];
    while let Some(l) = rx.recv().await {
        tracing::debug!("read {} from channel", l);
        if l == daemon::SEPERATOR {
            break;
        }
        done.push(l)
    }
    let mut downloading = Vec::with_capacity(8);
    while let Some(l) = rx.recv().await {
        tracing::debug!("read {} from channel", l);
        if l == daemon::TERMINATOR {
            break;
        }
        downloading.push(l);
    }
    crate::notify!("Queued"; content: "{}", queued_count);
    crate::notify!("Downloaded"; content: "{}", done.iter().format("\n"));
    crate::notify!("Downloading"; content: "{}", downloading.iter().format("\n"));
    Ok(())
}
