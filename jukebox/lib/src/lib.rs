mod arg_split;
pub mod prompt;
mod reconnect;
pub mod relay;
pub mod server;
#[cfg(feature = "jni_lib")]
pub mod android;

use std::{fmt::Display, io};

pub enum UiError {
    Io(io::Error),
    Closed,
}

pub type UiResult<'s, T = String> = Result<T, UiError>;

pub trait Ui {
    fn room_name(&mut self) -> UiResult;
    fn command(&mut self) -> UiResult;
    fn inform<I: Information>(&mut self, r: I);
}

fn print_result<T: Display, E: Display>(r: &Result<T, E>) {
    match r {
        Ok(s) => println!("{}", s),
        Err(e) => println!("\x1b[1;31mError:\x1b[0m\n{}", e),
    }
}

pub trait Information {
    fn info<F>(&self, f: F)
    where
        F: Fn(&dyn Display);
}

// impl<T> Information for T
// where
//     T: Display,
// {
//     fn info(&self) -> String {
//         self.to_string()
//     }
// }

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
            Err(ref e) => f(e),
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
