use mlib::players::PlayersDaemonOptions;

#[tokio::main]
async fn main() -> Result<(), mlib::Error> {
    mlib::players::start_daemon_if_running_as_daemon(PlayersDaemonOptions { with_video: true })
        .await?;
    if let Err(e) = mlib::players::all().await {
        eprintln!("failed to start music daemon: {e:?}");
    }
    Ok(())
}
