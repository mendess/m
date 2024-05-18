use std::{
    future::Future,
    io::{self, stdout, Write},
    pin::pin,
    thread,
    time::Duration,
};

use crate::{player_ctl, util::RawMode};
use crossterm::{
    cursor::{self, MoveTo},
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{Clear, ClearType},
    QueueableCommand,
};
use futures_util::{future::ready, join, Stream, StreamExt};
use mlib::{
    players::{self, event::OwnedLibMpvEvent, PlayerLink},
    queue::Queue,
};
use tokio::{sync::mpsc, time::timeout};

#[derive(Debug)]
struct PlaybackPosition {
    percent_position: Option<f64>,
    playback_time: Option<Duration>,
}

#[derive(Debug)]
enum UiUpdate {
    Title {
        title: String,
        total_time: f64,
        next: Option<String>,
    },
    Volume(f64),
    Pause {
        is_paused: bool,
    },
    ChapterName {
        title: String,
        total_time: f64,
    },
    ChapterNumber(usize),
    Position(PlaybackPosition),
    Quit,
    ClearChapter,
}

async fn input_task() {
    fn read() -> mpsc::Receiver<io::Result<event::Event>> {
        let (tx, rx) = mpsc::channel(100);
        thread::spawn(move || loop {
            if tx.is_closed() {
                break;
            }
            match event::poll(Duration::from_millis(100)) {
                Ok(true) => {
                    if tx.blocking_send(event::read()).is_err() {
                        break;
                    }
                }
                Ok(false) => {}
                Err(e) => {
                    // no need to check, we will break anyway
                    let _ = tx.blocking_send(Err(e));
                    break;
                }
            }
        });
        rx
    }
    let mut events = read();
    loop {
        let event = match events.recv().await {
            Some(Ok(e)) => e,
            Some(Err(_e)) => break,
            None => break,
        };
        use crossterm::event::KeyModifiers as Mod;
        match event {
            Event::Key(
                KeyEvent {
                    code: KeyCode::Char('q'),
                    modifiers: Mod::NONE,
                    ..
                }
                | KeyEvent {
                    code: KeyCode::Char('c' | 'd'),
                    modifiers: Mod::CONTROL,
                    ..
                },
            ) => break,
            Event::Key(KeyEvent {
                code: KeyCode::Char(c),
                modifiers,
                ..
            }) => {
                let _ = match (c, modifiers) {
                    ('p', _) => player_ctl::pause().await,
                    ('l', Mod::NONE) => player_ctl::next_file(1).await,
                    ('h', Mod::NONE) => player_ctl::prev_file(1).await,
                    ('j', Mod::NONE) => player_ctl::vd(2).await,
                    ('k', Mod::NONE) => player_ctl::vu(2).await,
                    ('h' | 'H', Mod::SHIFT) => player_ctl::prev(1).await,
                    ('l' | 'L', Mod::SHIFT) => player_ctl::next(1).await,
                    ('j' | 'J', Mod::SHIFT) | ('u', Mod::NONE) => player_ctl::back(2).await,
                    ('k' | 'K', Mod::SHIFT) | ('i', Mod::NONE) => player_ctl::frwd(2).await,
                    _ => Ok(()),
                };
            }
            _ => {}
        }
    }
}

async fn current_position() -> Option<PlaybackPosition> {
    async fn retry_until_positive<F, Fut>(f: F) -> Option<f64>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Option<f64>>,
    {
        for _ in 0..10 {
            let value = f().await?;
            if value >= 0.0 {
                return Some(value);
            }
        }
        None
    }
    let (percent_position, playback_time) = join!(
        retry_until_positive(|| async { players::percent_position().await.ok() }),
        retry_until_positive(|| async { players::playback_time().await.ok() }),
    );
    Some(PlaybackPosition {
        percent_position,
        playback_time: playback_time.map(Duration::from_secs_f64),
    })
}

async fn event_listener() -> impl Stream<Item = UiUpdate> {
    let event_stream = players::subscribe().await.unwrap();
    event_stream
        .filter_map(|r| ready(r.ok()))
        .filter_map(|ev| async move {
            match ev.event {
                OwnedLibMpvEvent::Shutdown => Some(UiUpdate::Quit),
                OwnedLibMpvEvent::FileLoaded | OwnedLibMpvEvent::PlaybackRestart => None,
                OwnedLibMpvEvent::Seek => Some(UiUpdate::Position(current_position().await?)),
                OwnedLibMpvEvent::PropertyChange { name, change, .. } => match name.as_str() {
                    "playlist-pos" => Some(UiUpdate::ClearChapter),
                    "media-title" => {
                        let title = change.into_string().ok()?;
                        let total_time = players::duration().await.ok()?;
                        let next = Queue::up_next(PlayerLink::current(), None)
                            .await
                            .ok()
                            .flatten();
                        Some(UiUpdate::Title {
                            title,
                            total_time,
                            next,
                        })
                    }
                    "volume" => {
                        let volume = change.into_double().ok()?;
                        Some(UiUpdate::Volume(volume))
                    }
                    "pause" => {
                        let is_paused = change.into_bool().ok()?;
                        Some(UiUpdate::Pause { is_paused })
                    }
                    "chapter-metadata" => {
                        let mut map = change.into_map().ok()?;
                        let title = map.remove("title")?.into_string().ok()?;
                        let total_time = players::duration().await.ok()?;
                        Some(UiUpdate::ChapterName { title, total_time })
                    }
                    "chapter" => {
                        let index = change.into_int().ok()?;
                        Some(UiUpdate::ChapterNumber(index as _))
                    }
                    _ => None,
                },
                _ => None,
            }
        })
}

pub async fn interactive() -> anyhow::Result<()> {
    let _guard = RawMode::enable()?;
    let (column, row) = cursor::position()?;
    crate::notify!("Loading....");
    let mut input_task = pin!(input_task());
    let mut ui_task = pin!(async {
        let mut event_listener = pin!(event_listener().await);
        let mut current = Queue::current(PlayerLink::current()).await.unwrap();
        loop {
            let r = stdout()
                .lock()
                .queue(MoveTo(column, row))
                .and_then(|s| s.queue(Clear(ClearType::FromCursorDown)))
                .and_then(|s| s.flush());
            match r {
                Ok(_) => crate::queue_ctl::display_current(&current, false).await,
                Err(e) => anyhow::Result::Err(e.into()),
            }
            .unwrap();
            let listen = timeout(Duration::from_secs(1), event_listener.next()).await;
            match listen {
                Err(_timedout) => {
                    if let Some(PlaybackPosition {
                        percent_position,
                        playback_time,
                    }) = current_position().await
                    {
                        current.progress = percent_position;
                        current.playback_time = playback_time;
                    }
                }
                Ok(Some(event)) => match event {
                    UiUpdate::ClearChapter => current.chapter = None,
                    UiUpdate::Title {
                        title,
                        total_time,
                        next,
                    } => {
                        current.title = title;
                        current.chapter = None;
                        current.duration = Duration::from_secs_f64(total_time);
                        current.next = next;
                    }
                    UiUpdate::Volume(volume) => current.volume = volume,
                    UiUpdate::Pause { is_paused } => current.playing = !is_paused,
                    UiUpdate::ChapterName { title, total_time } => {
                        current.chapter.get_or_insert_with(Default::default).1 = title;
                        current.duration = Duration::from_secs_f64(total_time);
                    }
                    UiUpdate::ChapterNumber(index) => {
                        current.chapter.get_or_insert_with(Default::default).0 = index;
                    }
                    UiUpdate::Position(PlaybackPosition {
                        percent_position,
                        playback_time,
                    }) => {
                        current.progress = percent_position;
                        current.playback_time = playback_time;
                    }
                    UiUpdate::Quit => break,
                },
                Ok(None) => {}
            }
        }
    });
    tokio::select! {
        _ = &mut input_task => {}
        _ = &mut ui_task => {}
    }
    Ok(())
}
