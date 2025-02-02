use std::{
    ffi::{OsStr, OsString},
    io,
    os::unix::ffi::OsStringExt,
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
use derive_more::derive::From;

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

#[must_use]
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

#[derive(Debug, From)]
pub enum GlobLibError {
    Iter(glob::GlobError),
    Pat(glob::PatternError),
}

pub async fn search_cache_for(
    dl_dir: &Path,
    link: &VideoLink,
) -> Result<Option<PathBuf>, GlobLibError> {
    let mut s = dl_dir.to_string_lossy().into_owned();
    s.push_str("/*=");
    s.push_str(link.id());
    s.push_str("=m.*");
    tokio::task::spawn_blocking(move || {
        tracing::debug!("searching cache using glob: {:?}", s);
        let file = glob::glob(&s).map(Paths::collect::<Vec<_>>);
        let mut files = match file {
            Ok(files) => files,
            Err(e) => return Err(e.into()),
        };
        let file = match files.pop() {
            Some(Ok(file)) if files.is_empty() => file,
            None => return Ok(None),
            Some(last) => {
                if !files.is_empty() {
                    tracing::warn!(
                        "glob {:?} matched multiple files: {:?}[{:?}]",
                        s,
                        files,
                        last
                    );
                }
                let last = match last {
                    Ok(last) => last,
                    Err(e) => return Err(e.into()),
                };
                if !files.is_empty() {
                    tracing::warn!("picking last one: {}", last.display());
                }
                last
            }
        };
        Ok(Some(file))
    })
    .await
    .unwrap()
}

pub async fn check_cache_ref(dl_dir: &Path, item: &mut Item) -> CheckCacheDecision {
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
    match search_cache_for(dl_dir, link).await {
        Ok(Some(file)) => *item = Item::File(file),
        Ok(None) => {
            tracing::debug!("song {:?} not found, deciding to download", link);
            return CheckCacheDecision::Download(link.clone());
        }
        Err(GlobLibError::Pat(pat)) => {
            tracing::error!(error = ?pat, "failed to parse glob pattern");
        }
        Err(GlobLibError::Iter(e)) => {
            tracing::error!(error = ?e, "failed to iterate files");
        }
    };
    CheckCacheDecision::Skip
}

pub struct GetDlPath<'v> {
    output_format: PathBuf,
    link: &'v VideoLink,
}

impl GetDlPath<'_> {
    pub async fn get(&self) -> Result<PathBuf, Error> {
        let o = OsStr::new;
        let mut output = Command::new("youtube-dl")
            .args([
                o("-o"),
                self.output_format.as_os_str(),
                o(self.link.as_str()),
                o("--print"),
                o("filename"),
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?
            .wait_with_output()
            .await?;
        if output.status.success() {
            while output.stdout.last() == Some(&b'\n') {
                output.stdout.pop();
            }
            Ok(PathBuf::from(OsString::from_vec(output.stdout)))
        } else {
            Err(YtdlError::NonZeroStatus {
                status_code: output.status,
                stderr: String::from_utf8(output.stderr)
                    .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
            }
            .into())
        }
    }
}

pub async fn download(
    dl_dir: PathBuf,
    link: &VideoLink,
    just_audio: bool,
) -> Result<GetDlPath<'_>, Error> {
    tokio::fs::create_dir_all(&dl_dir).await?;
    let mut output_format = dl_dir;
    output_format.push("%(title)s=%(id)s=m.%(ext)s");
    let mut cmd = Command::new("youtube-dl");
    if just_audio {
        cmd.arg("-x");
    }
    let o = OsStr::new;
    tracing::info!("downloading {}", link.as_str());
    let output = cmd
        .args([
            o("-o"),
            output_format.as_os_str(),
            o("--add-metadata"),
            o("--embed-chapters"),
            o(link.as_str()),
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?
        .wait_with_output()
        .await?;
    if output.status.success() {
        Ok(GetDlPath {
            output_format,
            link,
        })
    } else {
        Err(YtdlError::NonZeroStatus {
            status_code: output.status,
            stderr: String::from_utf8(output.stderr)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()),
        }
        .into())
    }
}
