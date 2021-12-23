use std::{io, path::PathBuf};

use futures_util::{Stream, TryStreamExt};
use tokio::fs;
use tokio_stream::wrappers::ReadDirStream;

use crate::{id_from_path, playlist::PlaylistIds};

fn dl_dir() -> Option<PathBuf> {
    let mut p = dirs::audio_dir()?;
    p.push("m");
    Some(p)
}

pub async fn clean_downloads(
    ids: &PlaylistIds,
) -> Result<impl Stream<Item = Result<PathBuf, io::Error>> + '_, crate::Error> {
    let files = fs::read_dir(dl_dir().ok_or(crate::Error::MusicDirNotFound)?).await?;
    Ok(
        ReadDirStream::new(files).try_filter_map(move |f| async move {
            if !f.metadata().await?.is_file() {
                return Ok(None);
            }
            let fname = f.file_name();
            let id = match id_from_path(&fname) {
                Some(id) => id,
                None => return Ok(None),
            };
            Ok(ids.contains(id).then(|| f.path()))
        }),
    )
}
