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
            cmd.arg(format!(
                "{}{}",
                if self.error { "Error: " } else { "" },
                triplets(&self.title.replace("\t", ""))
                    .map(|(s, _)| s)
                    .collect::<String>()
            ));
            if let Some(content) = &self.content {
                cmd.arg(
                    triplets(&content.replace("\t", ""))
                        .map(|(s, _)| s)
                        .collect::<String>(),
                );
            }
            cmd.spawn()?.wait().await?;
        } else {
            match try_color_stdout() {
                Some(mut t) => {
                    t.attr(term::Attr::Bold)?;
                    if self.error {
                        t.fg(term::color::RED)?;
                        t.write_all(b"Error: ")?;
                        t.fg(term::color::WHITE)?;
                    }
                    for (s, c) in triplets(&self.title) {
                        t.write_all(s.as_bytes())?;
                        match c {
                            "b" => t.fg(term::color::BLUE)?,
                            "r" | "w" => t.fg(term::color::WHITE)?,
                            _ => (),
                        }
                    }
                    t.write_all(b"\n")?;
                    t.reset()?;
                    if let Some(content) = &self.content {
                        for (s, c) in triplets(content) {
                            t.write_all(s.as_bytes())?;
                            match c {
                                "b" => t.fg(term::color::BLUE)?,
                                "r" | "w" => t.fg(term::color::WHITE)?,
                                _ => (),
                            }
                        }
                        t.write_all(b"\n")?;
                    }
                }
                None => {
                    if self.error {
                        print!("Error: ");
                    }
                    for (s, _) in triplets(&self.title) {
                        print!("{}", s);
                    }
                    println!();
                    if let Some(content) = &self.content {
                        for (s, _) in triplets(content) {
                            print!("{}", s);
                        }
                        println!();
                    }
                }
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

pub fn try_color_stdout() -> Option<Box<term::StdoutTerminal>> {
    atty::is(atty::Stream::Stdout).then(term::stdout).flatten()
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
