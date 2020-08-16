use std::{
    fmt::{self, Display},
    io::{self, stdin, stdout, Write},
};

#[derive(Debug, Default)]
pub struct Prompt {
    buf: String,
}

impl Prompt {
    pub fn p(&mut self, prompt: &str) -> io::Result<usize> {
        loop {
            self.buf.clear();
            print!("{} ", prompt);
            stdout().flush()?;
            let r = stdin().read_line(&mut self.buf);
            match self.buf.len() {
                1 => continue,   // Empty string, let's try again
                0 => println!(), // EOF, put a new line so the shell prompt appears on a new line
                _ => (),
            }
            self.buf.pop();
            break r;
        }
    }

    pub fn buf(&self) -> &str {
        &self.buf
    }
}

impl Display for Prompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.buf)
    }
}
