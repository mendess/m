#[cfg(feature = "jni_lib")]
pub mod android;
mod arg_split;
pub mod net;
pub mod prompt;
pub mod relay;
pub mod server;

use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Display},
    io,
    str::FromStr,
};

pub enum UiError {
    Io(io::Error),
    Invalid(String),
    Closed,
}

pub type UiResult<'s, T = String> = Result<T, UiError>;

pub trait Ui {
    fn room_name(&mut self) -> UiResult<RoomName>;
    fn command(&mut self) -> UiResult;
    fn inform<I: Information>(&mut self, r: I);
}

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
            Err($crate::UiError::Closed) => $k Ok(()),
            Err($crate::UiError::Io(e)) => $k Err(e.into()),
            Err($crate::UiError::Invalid(e)) => $k Err(
                ::std::io::Error::new(
                    ::std::io::ErrorKind::Other,
                    e,
                ).into()
            )
        }
    };
}

#[derive(
    Serialize, Deserialize, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Clone,
)]
pub struct RoomName {
    pub name: String,
}

impl FromStr for RoomName {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.contains("/") {
            Err("Room name can't contain '/'s")
        } else {
            Ok(Self { name: s.into() })
        }
    }
}

impl Display for RoomName {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.write_str(&self.name)
    }
}
