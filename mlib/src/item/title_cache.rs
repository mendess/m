use std::{
    io,
    path::{Path, PathBuf},
};

use base64::{engine::GeneralPurpose, Engine};

use super::{Search, VideoId};

async fn cache_path_for<S: AsRef<str> + ?Sized>(url: &S) -> PathBuf {
    let (path, _error) =
        namespaced_tmp::async_impl::in_user_tmp(&format!("m_title_cache/{}", url.as_ref())).await;
    path
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
