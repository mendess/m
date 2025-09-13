use std::{path::PathBuf, sync::Arc};

use arc_swap::ArcSwapOption;

static SOCKET_BASE_DIR_OVERRIDE: ArcSwapOption<PathBuf> = ArcSwapOption::const_empty();

pub async fn legacy_socket_for(index: usize) -> String {
    let socket_name = format!(".mpvsocket{index}");
    match &*SOCKET_BASE_DIR_OVERRIDE.load() {
        Some(base) => base.join(socket_name).display().to_string(),
        None => {
            let (path, e) = namespaced_tmp::async_impl::in_user_tmp(&socket_name).await;
            if let Some(e) = e {
                tracing::error!("failed to create socket dir: {:?}", e);
            }
            path.display().to_string()
        }
    }
}

pub fn override_legacy_socket_base_dir(new_base: PathBuf) {
    SOCKET_BASE_DIR_OVERRIDE.store(Some(Arc::new(new_base)));
}
