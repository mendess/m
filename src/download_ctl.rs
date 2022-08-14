use std::path::PathBuf;

use crate::download_ctl::daemon::Status;

use self::daemon::{Message, DAEMON};
use futures_util::StreamExt;
use itertools::Itertools;
use mlib::{
    downloaded::{check_cache, CheckCacheDecision},
    playlist::Playlist,
    Item,
};

mod daemon {
    use std::{
        collections::HashSet, num::NonZeroUsize, thread::available_parallelism, time::Duration,
    };

    use cli_daemon::Daemon;
    use futures_util::{stream::FuturesUnordered, StreamExt};
    use mlib::{downloaded, item::link::VideoLink, playlist::Playlist};
    use once_cell::sync::Lazy;
    use serde::{Deserialize, Serialize};
    use tokio::{
        sync::{mpsc, oneshot, Mutex},
        time::timeout,
    };
    use tracing::{error, info};

    use crate::config::DownloadFormat;

    #[derive(Serialize, Deserialize, Debug)]
    pub enum Message {
        Queue(VideoLink),
        Status,
    }

    #[derive(Serialize, Deserialize, Debug, Default, Clone)]
    pub struct Status {
        pub downloading: HashSet<VideoLink>,
        pub queued: HashSet<VideoLink>,
        pub done: Vec<VideoLink>,
        pub errored: Vec<VideoLink>,
    }
    impl Status {
        fn move_to_downloading(&mut self, l: &VideoLink) {
            let v = self
                .queued
                .take(l)
                .expect("I expected to find this value queued");
            self.downloading.insert(v);
        }

        fn move_to_done(&mut self, l: &VideoLink) {
            let v = self
                .downloading
                .take(l)
                .expect("I expected to find this value downloading");
            self.done.push(v);
        }

        fn move_to_errored(&mut self, l: &VideoLink) {
            let v = self
                .downloading
                .take(l)
                .expect("I expected to find this value downloading");
            self.errored.push(v);
        }
    }

    const ARG_0: &str = "into-the-m-verse";

    pub static DAEMON: Daemon<Message, Option<Status>> = Daemon::new(ARG_0);

    pub async fn start_daemon() -> anyhow::Result<()> {
        let builder = match DAEMON.build_daemon_process().await {
            None => return Ok(()),
            Some(b) => b,
        };

        let (tx, mut rx) = mpsc::channel::<VideoLink>(1000);
        let dl_dir = crate::util::dl_dir()?;

        static STATUS: Lazy<Mutex<Status>> = Lazy::new(Mutex::default);
        let paralellism = match available_parallelism().map(NonZeroUsize::get).unwrap_or(1) {
            1 => 1,
            x => x >> 1,
        };

        let (shutdown_send, shutdown_recv) = oneshot::channel();

        tokio::spawn(async move {
            let mut task_set = FuturesUnordered::new();

            loop {
                match timeout(Duration::from_secs(60), rx.recv()).await {
                    Ok(Some(l)) => {
                        STATUS.lock().await.move_to_downloading(&l);
                        tracing::info!(?l, "starting download task");
                        task_set.push(tokio::spawn({
                            let dl_dir = dl_dir.clone();
                            async move {
                                let result = downloaded::download(
                                    dl_dir.clone(),
                                    &l,
                                    crate::config::CONFIG.download_format == DownloadFormat::Audio,
                                )
                                .await;
                                match result {
                                    Ok(_) => {
                                        info!(?l, "downloaded");
                                        STATUS.lock().await.move_to_done(&l);
                                    }
                                    Err(e) => {
                                        let playlist = Playlist::load().await;

                                        let song = playlist.as_ref().ok().map(|pl| {
                                            pl.find_by_link(&l)
                                                .map(|s| s.name.as_str())
                                                .unwrap_or(l.as_str())
                                        });
                                        error!(?e, ?song, "error downloading link");
                                        STATUS.lock().await.move_to_errored(&l);
                                    }
                                }
                            }
                        }));

                        while task_set.len() >= paralellism {
                            let _ = task_set.next().await.unwrap();
                        }
                    }
                    Err(_) if !STATUS.lock().await.downloading.is_empty() => continue,
                    Ok(None) | Err(_) => break,
                }
            }
            while task_set.next().await.is_some() {}
            let _ = shutdown_send.send(());
        });

        match builder
            .with_shutdown(shutdown_recv)
            .run(move |message| {
                let tx = tx.clone();
                async move {
                    match message {
                        Message::Queue(l) => {
                            STATUS.lock().await.queued.insert(l.clone());
                            let _ = tx.send(l).await;
                            None
                        }
                        Message::Status => Some(STATUS.lock().await.clone()),
                    }
                }
            })
            .await? {}
    }
}

pub async fn daemon_status() -> anyhow::Result<()> {
    let Status {
        done,
        downloading,
        queued,
        errored,
    } = daemon::DAEMON
        .exchange(Message::Status)
        .await?
        .expect("daemon should have given me status");
    if !queued.is_empty() {
        crate::notify!("Queued"; content: "{}", queued.iter().format("\n"));
    }
    if !done.is_empty() {
        crate::notify!("Done"; content: "{}", done.iter().format("\n"));
    }
    if !downloading.is_empty() {
        crate::notify!("Downloading"; content: "{}", downloading.iter().format("\n"));
    }
    if !errored.is_empty() {
        crate::notify!("Errored"; content: "{}", errored.iter().format("\n"));
    }
    Ok(())
}

pub async fn check_cache_ref(path: PathBuf, item: &mut Item) {
    match mlib::downloaded::check_cache_ref(path, item).await {
        CheckCacheDecision::Skip => {}
        CheckCacheDecision::Download(l) => match DAEMON.exchange(Message::Queue(l)).await {
            Ok(None) => {}
            Ok(Some(_)) => panic!("server should not have given me a status"),
            Err(e) => crate::error!("failed to start myself: {:?}", e),
        },
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

pub use daemon::start_daemon as start_daemon_if_running_as_daemon;
