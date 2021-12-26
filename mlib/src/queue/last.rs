use crate::{playlist::Playlist, Error};
use std::{io, path::PathBuf};

fn path() -> Result<PathBuf, Error> {
    let mut path = Playlist::path()?;
    let mut name = path
        .file_name()
        .expect("playlist path to have a filename")
        .to_os_string();
    path.pop();
    name.push("_last_queue");
    path.push(name);
    Ok(path)
}

pub async fn fetch() -> Result<Option<usize>, Error> {
    let path = path()?;
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

pub async fn reset() -> Result<(), Error> {
    let path = path()?;
    if let Err(e) = tokio::fs::remove_file(&path).await {
        if e.kind() != io::ErrorKind::NotFound {
            return Err(e.into());
        }
    }
    Ok(())
}

pub async fn set(u: usize) -> Result<(), Error> {
    let path = path()?;
    tokio::fs::write(path, u.to_string().as_bytes()).await?;
    Ok(())
}
