#[cfg(feature = "jni_lib")]
pub mod android;
mod arg_split;
pub mod net;
pub mod prompt;
pub mod relay;
pub mod relay_clients;
pub mod server;

use serde::{Deserialize, Serialize};
use std::{
    fmt::{self, Display},
    str::FromStr,
};

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
