use std::{env, io, os::unix::ffi::OsStringExt};
use tokio::{process::Command, sync::OnceCell};

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
                    r#"pstree -s $$ | tr '\n' ' ' | grep -vEq "\\?|login|lemon|tmux|sshd|{}""#,
                    String::from_utf8_lossy(&t.into_vec())
                ))
                .spawn()?
                .wait()
                .await?;
            Ok(status.success())
        }

        static CURRENT: OnceCell<SessionKind> = OnceCell::const_new();

        *CURRENT
            .get_or_init(|| async {
                let session_kind_var = env::var_os("SESSION_KIND");
                if matches!(&session_kind_var, Some(v) if v == "gui") {
                    SessionKind::Gui
                } else if matches!(&session_kind_var, Some(v) if v == "cli" || v == "tui") {
                    SessionKind::Cli
                } else if matches!(called_from_gui().await, Ok(true)) {
                    SessionKind::Gui
                } else {
                    SessionKind::Cli
                }
            })
            .await
    }
}
