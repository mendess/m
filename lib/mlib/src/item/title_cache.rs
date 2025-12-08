use std::{
    io,
    path::{Path, PathBuf},
};

use base64::{Engine, engine::GeneralPurpose};

use super::{Search, VideoId};

async fn cache_path_for<S: AsRef<str> + ?Sized>(id: &S) -> PathBuf {
    const TITLE_CACHE_DIR: &str = "title-cache";
    let Some(cache_dir) = dirs::cache_dir() else {
        let (path, _error) = namespaced_tmp::async_impl::in_user_tmp(&format!(
            "m-{TITLE_CACHE_DIR}/{}",
            id.as_ref()
        ))
        .await;
        tracing::warn!(
            id = %id.as_ref(),
            "cache dir not present, using {} for title cache", path.display()
        );
        return path;
    };
    cache_dir.join("m").join(TITLE_CACHE_DIR).join(id.as_ref())
}

pub async fn get_by_vid_id(id: &VideoId) -> io::Result<Option<String>> {
    let path = cache_path_for(id).await;
    get_inner(&path).await
}

pub async fn put_by_vid_id(id: &VideoId, title: &str) -> io::Result<()> {
    let path = cache_path_for(id).await;
    put_inner(&path, title).await
}

const BASE64: GeneralPurpose = base64::engine::general_purpose::URL_SAFE;

pub async fn get_by_search(id: &Search) -> io::Result<Option<String>> {
    let id = BASE64.encode(id.as_str());
    let path = cache_path_for(&id).await;
    get_inner(&path).await
}

pub async fn put_by_search(id: &Search, title: &str) -> io::Result<()> {
    let id = BASE64.encode(id.as_str());
    let path = cache_path_for(&id).await;
    put_inner(&path, title).await
}

async fn get_inner(path: &Path) -> io::Result<Option<String>> {
    match tokio::fs::read(path).await {
        Ok(title) => String::from_utf8(title).map(Some).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

async fn put_inner(path: &Path, title: &str) -> io::Result<()> {
    tokio::fs::create_dir_all(&path.parent().unwrap()).await?;
    tokio::fs::write(path, title).await
}
