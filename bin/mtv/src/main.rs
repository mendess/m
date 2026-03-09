use mlib::players::{DaemonOptions, PlayersDaemonOptions};

#[tokio::main]
async fn main() -> Result<(), mlib::Error> {
    mlib::players::start_daemon_if_running_as_daemon(
        PlayersDaemonOptions { with_video: false },
        DaemonOptions {
            create_default_player: true,
        },
    )
    .await?;
    if let Err(e) = mlib::players::all().await {
        eprintln!("failed to start music daemon: {e:?}");
    }
    Ok(())
}
