use crate::RoomName;
use std::{
    fmt::{self, Display},
    io::{self, stdin, stdout, Write},
};

#[derive(Debug, Default)]
pub struct Prompt {
    buf: String,
    prompt_str: String,
    room_name: Option<RoomName>,
}

pub fn pretty_prompt() -> Prompt {
    Prompt::with_prompt_str("🎵>".into())
}

impl Prompt {
    pub fn with_prompt_str(prompt_str: String) -> Self {
        Self {
            prompt_str,
            ..Default::default()
        }
    }

    pub fn with_room_name<I: Into<Option<RoomName>>>(
        mut self,
        name: I,
    ) -> Self {
        self.room_name = name.into();
        self
    }

    fn p<'s, I: Into<Option<&'s str>>>(&mut self, msg: I) -> io::Result<usize> {
        let msg = msg.into();
        loop {
            self.buf.clear();
            match msg {
                Some(msg) => print!("{} ", msg),
                None => print!(
                    "{} {} ",
                    self.room_name
                        .as_ref()
                        .map(|s| s.name.as_str())
                        .unwrap_or(""),
                    self.prompt_str
                ),
            }
            stdout().flush()?;
            let s = stdin().read_line(&mut self.buf)?;
            match s {
                // Empty string, let's try again
                1 => continue,
                // EOF, put a new line so the shell
                // prompt appears on a new line
                0 => println!(),
                _ => (),
            }
            self.buf.pop();
            break Ok(s);
        }
    }

    pub fn buf(&self) -> &str {
        &self.buf
    }

    pub fn room_name(&self) -> Option<&RoomName> {
        self.room_name.as_ref()
    }

    pub fn ask_room_name(&mut self) -> PromptResult<RoomName> {
        io_to_uiresult(|| self.p("Input room name:"))?;
        let rn = self
            .buf
            .parse::<RoomName>()
            .map_err(|e| PromptError::Invalid(e.to_string()))?;
        self.room_name = Some(rn.clone());
        Ok(rn)
    }

    pub fn command(&mut self) -> PromptResult {
        io_to_uiresult(|| self.p(None)).map(|_| self.buf().into())
    }

    pub fn inform<I: Information>(&mut self, r: I) {
        r.info(|d| println!("{}", d));
    }
}

fn io_to_uiresult<'a, F: FnMut() -> io::Result<usize> + 'a>(
    mut f: F,
) -> PromptResult<'a, ()> {
    match f() {
        Ok(0) => Err(PromptError::Closed),
        Ok(_) => Ok(()),
        Err(e) => Err(PromptError::Io(e)),
    }
}

impl Display for Prompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.buf)
    }
}

pub enum PromptError {
    Io(io::Error),
    Invalid(String),
    Closed,
}

pub type PromptResult<'s, T = String> = Result<T, PromptError>;

pub trait Information {
    fn info<F>(&self, f: F)
    where
        F: Fn(&dyn Display);
}

impl<T, E> Information for Result<T, E>
where
    T: Display,
    E: Display,
{
    fn info<F>(&self, f: F)
    where
        F: Fn(&dyn Display),
    {
        match self {
            Ok(ref s) => f(s),
            Err(ref e) => {
                f(&"\x1b[1;31mError:\x1b[0m" as &dyn Display);
                f(e);
            }
        }
    }
}

impl<T, E> Information for &Result<T, E>
where
    T: Display,
    E: Display,
{
    fn info<F>(&self, f: F)
    where
        F: Fn(&dyn Display),
    {
        (*self).info(f)
    }
}

impl Information for &str {
    fn info<F: Fn(&dyn Display)>(&self, f: F) {
        f(self)
    }
}

impl Information for String {
    fn info<F: Fn(&dyn Display)>(&self, f: F) {
        f(self)
    }
}

#[macro_export]
macro_rules! try_prompt {
    ($e:expr) => { try_prompt!($e, return) };
    ($e:expr, $k:tt) => {
        match $e {
            Ok(r) => r,
            Err($crate::prompt::PromptError::Closed) => $k Ok(()),
            Err($crate::prompt::PromptError::Io(e)) => $k Err(e.into()),
            Err($crate::prompt::PromptError::Invalid(e)) => $k Err(
                ::std::io::Error::new(
                    ::std::io::ErrorKind::Other,
                    e,
                ).into()
            )
        }
    };
}
