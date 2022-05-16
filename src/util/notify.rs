use std::{
    io::{self, stdout, StdoutLock, Write},
    path::Path,
};

use crate::util::session_kind::SessionKind;
use crossterm::{
    cursor::MoveToNextLine,
    style::{Attribute, Color, Print, SetAttribute, SetForegroundColor},
    tty::IsTty,
    QueueableCommand,
};
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
        trait BoolPaint {
            fn paint<T: crossterm::Command>(
                self,
                stdout: &mut StdoutLock,
                cmd: T,
            ) -> io::Result<()>;
        }
        impl BoolPaint for bool {
            fn paint<T: crossterm::Command>(
                self,
                stdout: &mut StdoutLock,
                cmd: T,
            ) -> io::Result<()> {
                self.then(|| stdout.queue(cmd))
                    .map(|_| Ok(()))
                    .unwrap_or(Ok(()))
            }
        }
        fn print(stdout: &mut StdoutLock, s: &str) -> io::Result<()> {
            if crossterm::terminal::is_raw_mode_enabled()? {
                for line in s.split_inclusive('\n') {
                    if line.ends_with('\n') {
                        stdout.queue(Print(&line[..(line.len().saturating_sub(2))]))?;
                        stdout.queue(MoveToNextLine(1))?;
                    } else {
                        stdout.queue(Print(line))?;
                    }
                }
            } else {
                stdout.write_all(s.as_bytes())?;
            }
            Ok(())
        }
        match SessionKind::current().await {
            SessionKind::Cli if !self.force_notify => {
                let stdout = stdout();
                let mut stdout = stdout.lock();
                let is_tty = stdout.is_tty();
                is_tty.paint(&mut stdout, SetAttribute(Attribute::Bold))?;
                if self.error {
                    is_tty.paint(&mut stdout, SetForegroundColor(Color::Red))?;
                    stdout.queue(Print("Error: "))?;
                    is_tty.paint(&mut stdout, SetForegroundColor(Color::Reset))?;
                }
                for (s, c) in triplets(&self.title) {
                    print(&mut stdout, s)?;
                    if let Some(x) = match c {
                        "b" => Some(Color::Blue),
                        "w" => Some(Color::White),
                        "r" => Some(Color::Reset),
                        _ => None,
                    } {
                        is_tty.paint(&mut stdout, SetForegroundColor(x))?;
                    }
                }
                if crossterm::terminal::is_raw_mode_enabled()? {
                    stdout.queue(MoveToNextLine(1))?;
                } else {
                    stdout.write_all(b"\n")?;
                }
                is_tty.paint(&mut stdout, SetAttribute(Attribute::Reset))?;
                if let Some(content) = &self.content {
                    for (s, c) in triplets(content) {
                        print(&mut stdout, s)?;
                        if let Some(x) = match c {
                            "b" => Some(Color::Blue),
                            "w" => Some(Color::White),
                            "r" => Some(Color::Reset),
                            _ => None,
                        } {
                            is_tty.paint(&mut stdout, SetForegroundColor(x))?;
                        }
                    }
                    if crossterm::terminal::is_raw_mode_enabled()? {
                        stdout.queue(MoveToNextLine(1))?;
                    } else {
                        stdout.write_all(b"\n")?;
                    }
                }
                stdout.flush()?;
            }
            _ => {
                let mut cmd = Command::new("notify-send");
                if self.error {
                    cmd.args(["--urgency", "critical"]);
                }
                if let Some(img) = self.img {
                    cmd.arg("-i");
                    cmd.arg(img);
                }
                cmd.args(["-a", "m"]);
                cmd.arg(format!(
                    "{}{}",
                    if self.error { "Error: " } else { "" },
                    triplets(&self.title.replace('\t', ""))
                        .map(|(s, _)| s)
                        .collect::<String>()
                ));
                if let Some(content) = &self.content {
                    cmd.arg(
                        triplets(
                            &content
                                .replace('\t', "")
                                .replace('<', "&lt;")
                                .replace('>', "&gt;"),
                        )
                        .map(|(s, _)| s)
                        .collect::<String>(),
                    );
                }
                cmd.spawn()?.wait().await?;
            }
        }
        Ok(())
    }
}

pub fn triplets(s: &str) -> impl Iterator<Item = (&str, &str)> {
    Triplets { s }
}

struct Triplets<'s> {
    s: &'s str,
}

impl<'s> Iterator for Triplets<'s> {
    type Item = (&'s str, &'s str);
    fn next(&mut self) -> Option<Self::Item> {
        match self.s.find('§') {
            Some(i) => {
                let end_of_symbol = i + '§'.len_utf8();
                if end_of_symbol == self.s.len() {
                    let r = (self.s, "");
                    self.s = "";
                    Some(r)
                } else {
                    let r = (&self.s[..i], &self.s[end_of_symbol..(end_of_symbol + 1)]);
                    self.s = &self.s[(end_of_symbol + 1)..];
                    Some(r)
                }
            }
            None => {
                if self.s.is_empty() {
                    None
                } else {
                    let r = (self.s, "");
                    self.s = "";
                    Some(r)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn simple() {
        let t = triplets("ola §b test");

        assert_eq!(t.collect::<Vec<_>>(), [("ola ", "b"), (" test", "")]);
    }

    #[test]
    fn sym_at_start() {
        let t = triplets("§b test");
        assert_eq!(t.collect::<Vec<_>>(), [("", "b"), (" test", "")]);
    }

    #[test]
    fn sym_at_end() {
        let t = triplets("test §");
        assert_eq!(t.collect::<Vec<_>>(), [("test §", "")]);
    }
}
