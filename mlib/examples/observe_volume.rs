use mlib::socket::{cmds::MediaTitle as Prop, MpvSocket};
use tracing::dispatcher::set_global_default;
use tracing_log::LogTracer;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

fn init() {
    LogTracer::init().expect("Failed to set logger");

    let env_filter = if let Ok(e) = EnvFilter::try_from_default_env() {
        e
    } else {
        EnvFilter::new("trace")
    };

    let fmt = fmt::layer().event_format(fmt::format());

    let sub = Registry::default().with(env_filter).with(fmt);

    set_global_default(sub.into()).expect("Failed to set global default");
}

#[tokio::main]
async fn main() -> Result<(), mlib::Error> {
    init();
    let mut socket = MpvSocket::lattest().await?;

    socket
        .observe::<Prop, _>(|v| println!("prop is {}", v))
        .await
}
