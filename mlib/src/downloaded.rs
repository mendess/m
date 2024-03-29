use std::{
    io,
    path::{Path, PathBuf},
    process::Stdio,
};

use futures_util::{Stream, TryStreamExt};
use glob::Paths;
use tokio::{fs, process::Command};
use tokio_stream::wrappers::ReadDirStream;

use crate::{
    item::{id_from_path, link::VideoLink},
    playlist::{self, PlaylistIds},
    queue::Item,
    ytdl::YtdlError,
    Error,
};

pub async fn clean_downloads<P: AsRef<Path>>(
    dl_dir: P,
    ids: &PlaylistIds,
) -> Result<impl Stream<Item = Result<PathBuf, io::Error>> + '_, crate::Error> {
    let files = fs::read_dir(dl_dir).await?;
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
            Ok((!ids.contains(id)).then(|| f.path()))
        }),
    )
}

pub enum CheckCacheDecision {
    Download(VideoLink),
    Skip,
}

pub async fn is_in_cache(dl_dir: &Path, link: &VideoLink) -> bool {
    let mut s = dl_dir.to_string_lossy().into_owned();
    s.push_str("/*=");
    s.push_str(link.id());
    s.push_str("=m.*");
    tokio::task::spawn_blocking(move || {
        tracing::debug!("searching cache using glob: {:?}", s);
        let file = glob::glob(&s).map(Paths::collect::<Vec<_>>);
        let mut files = match file {
            Ok(files) => files,
            Err(e) => {
                tracing::error!("parsing glob pattern {:?}: {:?}", s, e);
                return false;
            }
        };
        matches!(files.pop(), Some(Ok(_)) if files.is_empty())
    })
    .await
    .unwrap()
}

pub async fn check_cache_ref(dl_dir: PathBuf, item: &mut Item) -> CheckCacheDecision {
    enum R {
        F(PathBuf),
        Error,
        None,
    }
    let link = match item {
        Item::Link(l) => match l.as_video() {
            Some(v) => v,
            None => return CheckCacheDecision::Skip,
        },
        _ => return CheckCacheDecision::Skip,
    };
    if !matches!(playlist::find_song(link.id()).await, Ok(Some(_))) {
        return CheckCacheDecision::Skip;
    }
    let mut s = dl_dir.to_string_lossy().into_owned();
    s.push_str("/*=");
    s.push_str(link.id());
    s.push_str("=m.*");
    let file = tokio::task::spawn_blocking(move || {
        tracing::debug!("searching cache using glob: {:?}", s);
        let file = glob::glob(&s).map(Paths::collect::<Vec<_>>);
        let mut files = match file {
            Ok(files) => files,
            Err(e) => {
                tracing::error!("parsing glob pattern {:?}: {:?}", s, e);
                return R::Error;
            }
        };
        let file = match files.pop() {
            Some(Ok(file)) if files.is_empty() => file,
            None => {
                return R::None;
            }
            Some(last) => {
                tracing::warn!(
                    "glob {:?} matched multiple files: {:?}[{:?}]",
                    s,
                    files,
                    last
                );
                let Ok(last) = last else {
                    return R::Error;
                };
                tracing::warn!("picking last one: {}", last.display());
                last
            }
        };
        R::F(file)
    })
    .await
    .unwrap();
    match file {
        R::F(file) => *item = Item::File(file),
        R::None => {
            tracing::debug!("song {:?} not found, deciding to download", link);
            return CheckCacheDecision::Download(link.clone());
        }
        _ => {}
    };
    CheckCacheDecision::Skip
}

pub async fn download(dl_dir: PathBuf, link: &VideoLink, just_audio: bool) -> Result<(), Error> {
    let mut output_format = dl_dir;
    output_format.push("%(title)s=%(id)s=m.%(ext)s");
    let mut cmd = Command::new("youtube-dl");
    if just_audio {
        cmd.arg("-x");
    }
    let output = cmd
        .args([
            "-o",
            &*output_format.to_string_lossy(),
            "--add-metadata",
            link.as_str(),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?
        .wait_with_output()
        .await?;
    if output.status.success() {
        Ok(())
    } else {
        Err(YtdlError::NonZeroStatus {
            status_code: output.status,
            stderr: String::from_utf8(output.stderr)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
        }
        .into())
    }
}
