mod socket_server;
mod web_server;
mod jukebox;
mod socket_channel;

use tokio::io;

pub async fn start(port: u16) -> io::Result<()> {
    let handle = tokio::spawn(socket_server::start(port));
    web_server::start(port + 1, &*socket_server::ROOMS).await?;
    handle.await??;
    Ok(())
}
