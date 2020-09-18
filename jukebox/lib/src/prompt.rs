use crate::{Information, RoomName, Ui, UiError, UiResult};
use std::{
    fmt::{self, Display},
    io::{self, stdin, stdout, Write},
};

#[derive(Debug, Default)]
pub struct Prompt {
    buf: String,
    prompt_str: String,
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

    pub fn p(&mut self, msg: &str) -> io::Result<usize> {
        prompt(&mut self.buf, msg)
    }

    pub fn buf(&self) -> &str {
        &self.buf
    }
}

fn prompt(buf: &mut String, msg: &str) -> io::Result<usize> {
    loop {
        buf.clear();
        print!("{} ", msg);
        stdout().flush()?;
        let s = stdin().read_line(buf)?;
        match s {
            1 => continue,   // Empty string, let's try again
            0 => println!(), // EOF, put a new line so the shell
            // prompt appears on a new line
            _ => (),
        }
        buf.pop();
        break Ok(s);
    }
}

fn prompt_conv<'a>(buf: &mut String, msg: &str) -> UiResult<'a, ()> {
    match prompt(buf, msg) {
        Ok(0) => Err(UiError::Closed),
        Ok(_) => Ok(()),
        Err(e) => Err(UiError::Io(e)),
    }
}

impl Display for Prompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.buf)
    }
}

impl Ui for Prompt {
    fn room_name(&mut self) -> UiResult<RoomName> {
        let e = prompt_conv(&mut self.buf, "Input room name:");
        if e.is_ok() {
            self.prompt_str = format!("{} 🎵>", self.buf());
        }
        self.buf()
            .parse::<RoomName>()
            .map_err(|e| UiError::Invalid(e.to_string()))
    }

    fn command(&mut self) -> UiResult {
        match prompt_conv(&mut self.buf, &self.prompt_str) {
            Ok(_) => Ok(self.buf().into()),
            Err(e) => Err(e),
        }
    }

    fn inform<I: Information>(&mut self, r: I) {
        r.info(|d| println!("{}", d));
        // crate::print_result(r)
    }
}
