use crate::{socket::MpvSocket, Error};
use std::{
    io,
    path::PathBuf,
    time::{Duration, SystemTime},
};

fn path<S>(socket: &MpvSocket<S>) -> PathBuf {
    let mut path = socket.path().to_owned();
    let mut name = path
        .file_name()
        .expect("playlist path to have a filename")
        .to_os_string();
    path.pop();
    name.push("_last_queue");
    path.push(name);
    path
}

pub async fn fetch<S>(socket: &MpvSocket<S>) -> Result<Option<usize>, Error> {
    const THREE_HOURS: Duration = Duration::from_secs(60 * 60 * 3);

    let path = path(socket);
    let now = SystemTime::now();
    tracing::debug!(?path, "getting m_time on last queue file");
    let modified = match tokio::fs::metadata(&path).await.and_then(|r| r.modified()) {
        Ok(m_time) => m_time,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    tracing::debug!(?modified, ?path, "got m_time on last queue file");
    if (modified.duration_since(now).unwrap_or_default()) > THREE_HOURS
        || modified < socket.created_at().await?
    {
        reset(socket).await?;
        Ok(None)
    } else {
        match tokio::fs::read_to_string(&path).await {
            Ok(s) => match s.trim().parse() {
                Ok(n) => Ok(Some(n)),
                Err(_) => {
                    tracing::error!("failed to parse last queue, file corrupted? '{:?}'", path);
                    Ok(None)
                }
            },
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

pub async fn reset<S>(socket: &MpvSocket<S>) -> Result<(), Error> {
    let path = path(socket);
    if let Err(e) = tokio::fs::remove_file(&path).await {
        if e.kind() != io::ErrorKind::NotFound {
            return Err(e.into());
        }
    }
    Ok(())
}

pub async fn set<S>(socket: &MpvSocket<S>, u: usize) -> Result<(), Error> {
    let path = path(socket);
    tokio::fs::write(path, u.to_string().as_bytes()).await?;
    Ok(())
}
