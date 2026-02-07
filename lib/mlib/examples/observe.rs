use std::future::ready;

use futures_util::StreamExt;
use mlib::players;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt as _,
};

fn init() {
    tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with(fmt::layer().pretty())
        .init();
}

#[tokio::main]
async fn main() -> Result<(), mlib::Error> {
    init();
    players::start_daemon_if_running_as_daemon(
        Default::default(),
        Default::default(),
    )
    .await?;
    players::subscribe()
        .await?
        .for_each(|e| {
            tracing::info!(event = ?e, "new event");
            ready(())
        })
        .await;
    Ok(())
}
