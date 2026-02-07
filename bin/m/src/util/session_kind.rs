use std::{env, io};
use tokio::sync::OnceCell;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SessionKind {
    #[default]
    Cli,
    Gui,
}

impl SessionKind {
    pub async fn current() -> Self {
        async fn deduce_kind() -> io::Result<SessionKind> {
            let t = match env::var_os("TERMINAL") {
                Some(t) => t,
                None => return Ok(SessionKind::Cli),
            };

            let mut pid = std::process::id() as i32;
            while {
                // read PPID from /proc/<pid>/stat (field 4)
                let stat =
                    tokio::fs::read_to_string(format!("/proc/{pid}/stat"))
                        .await?;
                pid = stat.split_whitespace().nth(3).unwrap().parse().unwrap();
                pid != 1
            } {
                // read /proc/<pid>/exe symlink
                let exe =
                    tokio::fs::read_link(format!("/proc/{pid}/exe")).await?;
                let o = |s| std::ffi::OsStr::new(s);
                fn contains(haystack: &[u8], needle: &[u8]) -> bool {
                    haystack.windows(needle.len()).any(|w| w == needle)
                }
                let called_from_terminal =
                    [o("login"), o("lemon"), o("tmux"), o("sshd"), &t]
                        .into_iter()
                        .any(|p| {
                            contains(
                                exe.as_os_str().as_encoded_bytes(),
                                p.as_encoded_bytes(),
                            )
                        });
                if called_from_terminal {
                    return Ok(SessionKind::Cli);
                }
            }

            Ok(SessionKind::Gui)
        }

        static CURRENT: OnceCell<SessionKind> = OnceCell::const_new();

        *CURRENT
            .get_or_init(|| async {
                let session_kind_var = env::var_os("SESSION_KIND");
                if matches!(&session_kind_var, Some(v) if v == "gui") {
                    SessionKind::Gui
                } else if matches!(&session_kind_var, Some(v) if v == "cli" || v == "tui") {
                    SessionKind::Cli
                } else {
                    deduce_kind().await.unwrap_or_default()
                }
            })
            .await
    }
}
