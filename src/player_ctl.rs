use super::arg_parse::Amount;

use mlib::socket::{cmds, MpvSocket};

use crate::notify;

pub async fn quit() -> anyhow::Result<()> {
    Ok(fire("quit").await?)
}

pub async fn pause() -> anyhow::Result<()> {
    Ok(fire("cycle pause").await?)
}

pub async fn vu(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(fire(format!("add volume {}", amount.unwrap_or(2))).await?)
}

pub async fn vd(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(fire(format!("add volume -{}", amount.unwrap_or(2))).await?)
}

pub async fn toggle_video() -> anyhow::Result<()> {
    Ok(fire("cycle vid").await?)
}

pub async fn next_file(Amount { amount }: Amount) -> anyhow::Result<()> {
    let mut socket = MpvSocket::lattest().await?;
    for _ in 0..amount.unwrap_or(1) {
        socket.fire("playlist-next").await?;
    }
    Ok(())
}

pub async fn prev_file(Amount { amount }: Amount) -> anyhow::Result<()> {
    let mut socket = MpvSocket::lattest().await?;
    for _ in 0..amount.unwrap_or(1) {
        socket.fire("playlist-prev").await?;
    }
    Ok(())
}

pub async fn frwd(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(fire(format!("seek {}", amount.unwrap_or(10))).await?)
}

pub async fn back(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(fire(format!("seek -{}", amount.unwrap_or(10))).await?)
}

pub async fn next(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(fire(format!("add chapter {}", amount.unwrap_or(1))).await?)
}

pub async fn prev(Amount { amount }: Amount) -> anyhow::Result<()> {
    Ok(fire(format!("add chapter -{}", amount.unwrap_or(1))).await?)
}

pub async fn shuffle() -> anyhow::Result<()> {
    Ok(MpvSocket::lattest()
        .await?
        .execute(cmds::QueueShuffle)
        .await?)
}

pub async fn toggle_loop() -> anyhow::Result<()> {
    let mut socket = MpvSocket::lattest().await?;
    let looping = match socket.compute(cmds::QueueIsLooping).await? {
        cmds::LoopStatus::Inf => false,
        cmds::LoopStatus::No => true,
        _ => false,
    };
    socket.execute(cmds::QueueLoop(looping)).await?;
    if looping {
        notify!("now looping");
    } else {
        notify!("not looping");
    }
    Ok(())
}

async fn fire<S: AsRef<[u8]>>(c: S) -> anyhow::Result<()> {
    Ok(MpvSocket::lattest().await?.fire(c).await?)
}
