use async_once::AsyncOnce;
use std::{env, io, os::unix::ffi::OsStringExt};
use structopt::lazy_static::lazy_static;
use tokio::process::Command;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionKind {
    Cli,
    Gui,
}

impl SessionKind {
    pub async fn current() -> Self {
        async fn called_from_gui() -> io::Result<bool> {
            let t = match env::var_os("TERMINAL") {
                Some(t) => t,
                None => return Ok(false),
            };
            let status = Command::new("bash")
                .arg("-c")
                .arg(format!(
                    r#"pstree -s $$ | tr '\n' ' ' | grep -vEq "\\?|login|lemon|tmux|{}""#,
                    String::from_utf8_lossy(&t.into_vec())
                ))
                .spawn()?
                .wait()
                .await?;
            Ok(status.success())
        }

        lazy_static! {
            static ref CURRENT: AsyncOnce<SessionKind> = AsyncOnce::new(async {
                if matches!(env::var_os("SESSION_KIND"), Some(v) if v == "gui")
                    || matches!(called_from_gui().await, Ok(true))
                {
                    SessionKind::Gui
                } else {
                    SessionKind::Cli
                }
            });
        }

        *CURRENT.get().await
    }
}
