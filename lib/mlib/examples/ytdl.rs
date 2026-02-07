use mlib::{item::link::ChannelLink, ytdl::YtdlBuilder};
use tokio_stream::StreamExt as _;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    EnvFilter, fmt, layer::SubscriberExt, util::SubscriberInitExt as _,
};

#[tokio::main]
async fn main() {
    {
        tracing_subscriber::registry()
            .with(
                EnvFilter::builder()
                    .with_default_directive(LevelFilter::INFO.into())
                    .from_env_lossy(),
            )
            .with(fmt::layer().pretty())
            .init();
    }

    let c = "https://www.youtube.com/@iolandamusic/releases"
        .parse::<ChannelLink>()
        .unwrap();

    let mut channel = YtdlBuilder::new(&c)
        .get_title()
        .request_channel()
        .await
        .unwrap();
    while let Some(x) = channel.next().await {
        let video = x.unwrap();
        println!("video: {} :: {}", video.id().as_str(), video.title_ref());
    }
}
