use mlib::{item::link::ChannelLink, ytdl::YtdlBuilder};
use tokio_stream::StreamExt as _;
use tracing::dispatcher::set_global_default;
use tracing_log::LogTracer;
use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter, Registry};

#[tokio::main]
async fn main() {
    {
        LogTracer::init().expect("Failed to set logger");

        let env_filter = if let Ok(e) = EnvFilter::try_from_default_env() {
            e
        } else {
            EnvFilter::new("debug")
        };

        let fmt = fmt::layer().with_writer(std::io::stderr).pretty();

        let sub = Registry::default().with(env_filter).with(fmt);

        set_global_default(sub.into()).expect("Failed to set global default");
    }

    let c = "https://www.youtube.com/@iolandamusic/releases"
        .parse::<ChannelLink>()
        .unwrap();

    let mut channel = YtdlBuilder::new(&c).get_title().request_channel().unwrap();
    while let Some(x) = channel.next().await {
        let video = x.unwrap();
        println!("video: {} :: {}", video.id().as_str(), video.title_ref());
    }
}
