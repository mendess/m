mod arg_split;
pub mod prompt;
mod reconnect;
pub mod relay;
pub mod server;

use std::{fmt::Display, io};

pub enum UiError {
    Io(io::Error),
    Closed,
}

pub type UiResult<'s, T = &'s str> = Result<T, UiError>;

pub trait Ui {
    fn room_name(&mut self) -> UiResult;
    fn command(&mut self) -> UiResult;
    fn inform<T: Display, E: Display>(&mut self, r: &Result<T, E>);
}

fn print_result<T: Display, E: Display>(r: &Result<T, E>) {
    match r {
        Ok(s) => println!("{}", s),
        Err(e) => println!("\x1b[1;31mError:\x1b[0m\n{}", e),
    }
}
