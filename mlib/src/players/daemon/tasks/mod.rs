use super::SharedPlayersDaemon;
use futures_util::join;

pub mod last_queue_monitor;
#[cfg(feature = "mpris")]
pub mod mpris;
pub mod preemptive_dl;
#[cfg(feature = "statistics")]
pub mod statistics;

pub async fn register_global_tasks(players: SharedPlayersDaemon) {
    #[cfg(feature = "mpris")]
    let signal_mpris_events = {
        let players = players.clone();
        // do it like this so that the await on the "new_with_all" function can't block this
        // from calling "run_with_events".
        async move {
            match mpris_server::Server::new_with_all("m", mpris::MprisPlayer::new(players.clone()))
                .await
            {
                Ok(server) => {
                    mpris::signal_mpris_events(server, super::event_stream(players).await).await
                }
                Err(e) => {
                    tracing::error!(?e, "failed to initialize mpris server");
                }
            };
        }
    };
    #[cfg(not(feature = "mpris"))]
    let signal_mpris_events = std::future::ready(());
    #[cfg(feature = "statistics")]
    let stats_task = statistics::register_statistics_listener(super::event_stream(players).await);
    #[cfg(not(feature = "statistics"))]
    let stats_task = std::future::ready(());

    join!(signal_mpris_events, stats_task);
}
