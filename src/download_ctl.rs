use futures_util::{stream::FuturesUnordered, StreamExt};
use mlib::{downloaded, Link};

use crate::error;

pub async fn download(links: impl IntoIterator<Item = String>) -> anyhow::Result<()> {
    let mut task_set = FuturesUnordered::new();
    let dl_dir = crate::util::dl_dir()?;

    for l in links {
        match Link::from_url(l) {
            Err(e) => {
                error!("invalid url: {:?}", e);
            }
            Ok(l) => {
                tracing::debug!("downloading {}", l);
                task_set.push(downloaded::download(dl_dir.clone(), l));
            }
        }

        while task_set.len() > 8 {
            match task_set.next().await.unwrap() {
                Err(e) => error!("error downloading link: {:?}", e),
                Ok(l) => tracing::info!("downloaded {}", l),
            }
        }
    }
    Ok(())
}
