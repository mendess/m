use std::{io, path::PathBuf};

use super::VideoLink;

async fn cache_path_for(url: &VideoLink) -> PathBuf {
    let (path, _error) =
        namespaced_tmp::async_impl::in_user_tmp(&format!("m_title_cache/{}", url.id().as_str()))
            .await;
    path
}

pub async fn get(url: &VideoLink) -> io::Result<Option<String>> {
    let path = cache_path_for(url).await;
    match tokio::fs::read(path).await {
        Ok(title) => String::from_utf8(title).map(Some).map_err(io::Error::other),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn put(url: &VideoLink, title: &str) -> io::Result<()> {
    let path = cache_path_for(url).await;
    tokio::fs::create_dir_all(&path.parent().unwrap()).await?;
    tokio::fs::write(path, title).await
}
