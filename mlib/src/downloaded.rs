use std::{io, path::PathBuf, process::Stdio};

use futures_util::{Stream, TryStreamExt};
use glob::Paths;
use tokio::{fs, process::Command};
use tokio_stream::wrappers::ReadDirStream;

use crate::{id_from_path, playlist::PlaylistIds, queue::Item, Error, Link};

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

pub async fn check_cache(link: Link) -> Item {
    tokio::task::spawn_blocking(move || {
        let dl_dir = match dl_dir() {
            Some(d) => d,
            None => return Item::Link(link),
        };
        let mut s = dl_dir.to_string_lossy().into_owned();
        s.push_str("/*");
        s.push_str(link.id());
        s.push_str("=m.*");
        let file = glob::glob(&s).map(Paths::collect::<Vec<_>>);
        let mut files = match file {
            Ok(files) => files,
            Err(e) => {
                tracing::error!("parsing glob pattern {:?}: {:?}", s, e);
                return Item::Link(link);
            }
        };
        let file = match files.pop() {
            Some(Ok(file)) if files.is_empty() => file,
            None => {
                tracing::debug!("song {:?} not found, downloading", link);
                tokio::spawn(download(link.clone()));
                return Item::Link(link);
            }
            Some(last) => {
                tracing::warn!(
                    "glob {:?} matched multiple files: {:?} + [{:?}]",
                    s,
                    files,
                    last
                );
                return Item::Link(link);
            }
        };
        Item::File(file)
    })
    .await
    .unwrap()
}

pub async fn download(link: Link) -> Result<(), Error> {
    let mut output_format = dl_dir().ok_or(Error::MusicDirNotFound)?;
    output_format.push("%(title)s=%(id)s=m.%(ext)s");
    Command::new("youtube-dl")
        .arg("-o")
        .arg(&*output_format.to_string_lossy())
        .arg("--add-metadata")
        .arg(&link.0)
        // .stdout(Stdio::null())
        .spawn()?
        .wait()
        .await?;
    Ok(())
}
