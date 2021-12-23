use std::{fmt, io, path::Path};

use crate::session_kind::SessionKind;
use tokio::process::Command;

#[macro_export]
macro_rules! notify {
    (
        $($fmt:expr),*$(,)?
        $(; content: $($content:expr),*$(,)?)?
        $(; img: $img:expr)?
    ) => {{
        $crate::notify::Notify::new(::std::format_args!($($fmt),*))
        $(
            .content(::std::format_args!($($content),*))
        )*
        $(
            .img($img)
        )*
            .notify().await?
    }}
}

#[macro_export]
macro_rules! error {
    (
        $($fmt:expr),*
        $(; content: $($content:expr),*$(,)?)?
        $(; img: $img:expr)?
    ) => {{
        $crate::notify::Notify::new(::std::format_args!($($fmt),*))
            .error()
        $(
            .content(::std::format_args!($($content),*))
        )*
        $(
            .img($img)
        )*
            .notify().await?
    }}
}

pub struct Notify<'title, 'content, 'path> {
    title: fmt::Arguments<'title>,
    error: bool,
    content: Option<fmt::Arguments<'content>>,
    img: Option<&'path Path>,
}

impl<'title, 'content, 'path> Notify<'title, 'content, 'path> {
    pub fn new(title: fmt::Arguments<'title>) -> Self {
        Self {
            title,
            error: false,
            content: None,
            img: None,
        }
    }

    pub fn error(&mut self) -> &mut Self {
        self.error = true;
        self
    }

    pub fn content(&mut self, content: fmt::Arguments<'content>) -> &mut Self {
        self.content = Some(content);
        self
    }

    pub fn img(&mut self, img: &'path Path) -> &mut Self {
        self.img = Some(img);
        self
    }

    pub async fn notify(&self) -> io::Result<()> {
        match SessionKind::current().await {
            SessionKind::Cli => match atty::is(atty::Stream::Stdout).then(term::stdout).flatten() {
                Some(mut t) => {
                    t.attr(term::Attr::Bold)?;
                    if self.error {
                        t.fg(term::color::RED)?;
                        t.write_all(b"Error: ")?;
                        t.fg(term::color::WHITE)?;
                    }
                    t.write_fmt(self.title)?;
                    t.write_all(b"\n")?;
                    t.reset()?;
                    if let Some(content) = self.content {
                        t.write_fmt(content)?;
                        t.write_all(b"\n")?;
                    }
                }
                None => {
                    if self.error {
                        print!("Error: ");
                    }
                    println!("{}", self.title);
                    if let Some(content) = self.content {
                        println!("{}", content);
                    }
                }
            },
            SessionKind::Gui => {
                println!("fooooooo");
                let mut cmd = Command::new("notify-send");
                if self.error {
                    cmd.arg("--urgency");
                    cmd.arg("critical");
                }
                if let Some(img) = self.img {
                    cmd.arg("-i");
                    cmd.arg(img);
                }
                cmd.arg("-a");
                cmd.arg("m");
                cmd.arg(format!("{}", self.title));
                if let Some(content) = self.content {
                    cmd.arg(format!("{}", content));
                }
                cmd.spawn()?.wait().await?;
            }
        }
        Ok(())
    }
}
