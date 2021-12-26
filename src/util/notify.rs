use std::{io, path::Path};

use crate::util::session_kind::SessionKind;
use tokio::process::Command;

#[macro_export]
macro_rules! notify {
    (
        $($fmt:expr),*$(,)?
        $(; content: $($content:expr),*$(,)?)?
        $(; img: $img:expr)?
        $(; force_notify: $force_notify:expr)?
    ) => {{
        $crate::util::notify::Notify::new(::std::format!($($fmt),*))
        $(
            .content(::std::format!($($content),*))
        )*
        $(
            .img($img)
        )*
        $(
            .force_notify($force_notify)
        )*
            .notify().await
            .unwrap()
    }}
}

#[macro_export]
macro_rules! error {
    (
        $($fmt:expr),*
        $(; content: $($content:expr),*$(,)?)?
        $(; img: $img:expr)?
        $(; force_notify: $force_notify:expr)?
    ) => {{
        $crate::util::notify::Notify::new(::std::format!($($fmt),*))
            .error()
        $(
            .content(::std::format!($($content),*))
        )*
        $(
            .img($img)
        )*
        $(
            .force_notify($force_notify)
        )*
            .notify().await
            .unwrap()
    }}
}

pub struct Notify<'path> {
    title: String,
    error: bool,
    content: Option<String>,
    img: Option<&'path Path>,
    force_notify: bool,
}

impl<'path> Notify<'path> {
    pub fn new(title: String) -> Self {
        Self {
            title,
            error: false,
            content: None,
            img: None,
            force_notify: false,
        }
    }

    pub fn error(&mut self) -> &mut Self {
        self.error = true;
        self
    }

    pub fn content(&mut self, content: String) -> &mut Self {
        self.content = Some(content);
        self
    }

    pub fn img(&mut self, img: &'path Path) -> &mut Self {
        self.img = Some(img);
        self
    }

    pub fn force_notify(&mut self, b: bool) -> &mut Self {
        self.force_notify = b;
        self
    }

    pub async fn notify(&self) -> io::Result<()> {
        if self.force_notify || SessionKind::current().await == SessionKind::Gui {
            let mut cmd = Command::new("notify-send");
            if self.error {
                cmd.args(["--urgency", "critical"]);
            }
            if let Some(img) = self.img {
                cmd.arg("-i");
                cmd.arg(img);
            }
            cmd.args(["-a", "m"]);
            cmd.arg(&self.title);
            if let Some(content) = &self.content {
                cmd.arg(&content);
            }
            cmd.spawn()?.wait().await?;
        } else {
            match atty::is(atty::Stream::Stdout).then(term::stdout).flatten() {
                Some(mut t) => {
                    t.attr(term::Attr::Bold)?;
                    if self.error {
                        t.fg(term::color::RED)?;
                        t.write_all(b"Error: ")?;
                        t.fg(term::color::WHITE)?;
                    }
                    t.write_all(self.title.as_bytes())?;
                    t.write_all(b"\n")?;
                    t.reset()?;
                    if let Some(content) = &self.content {
                        t.write_all(content.as_bytes())?;
                        t.write_all(b"\n")?;
                    }
                }
                None => {
                    if self.error {
                        print!("Error: ");
                    }
                    println!("{}", self.title);
                    if let Some(content) = &self.content {
                        println!("{}", content);
                    }
                }
            }
        }
        Ok(())
    }
}
