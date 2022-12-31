use futures_util::StreamExt;
use mlib::players;
use tracing::dispatcher::set_global_default;
use tracing_log::LogTracer;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

fn init() {
    LogTracer::init().expect("Failed to set logger");

    let env_filter = if let Ok(e) = EnvFilter::try_from_default_env() {
        e
    } else {
        EnvFilter::new("info")
    };

    let fmt = fmt::layer().event_format(fmt::format());

    let sub = Registry::default().with(env_filter).with(fmt);

    set_global_default(sub.into()).expect("Failed to set global default");
}

#[tokio::main]
async fn main() -> Result<(), mlib::Error> {
    init();
    players::start_daemon_if_running_as_daemon().await?;
    players::subscribe()
        .await?
        .for_each(|e| async move { println!("{e:?}") })
        .await;
    Ok(())
}
