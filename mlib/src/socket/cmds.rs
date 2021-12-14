use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use self::command::Command;

#[derive(Serialize, Debug)]
#[serde(untagged)]
pub enum Value<'s> {
    Str(&'s str),
    String(String),
    Path(&'s Path),
    Bool(bool),
    Float(f64),
    Int(i64),
}

pub(crate) mod command {
    use super::Value;
    use serde::de::DeserializeOwned;

    pub trait Command<const N: usize> {
        type Output: DeserializeOwned;
        fn cmd(&self) -> [Value<'_>; N];
    }
}

pub struct Pause;

impl Command<3> for Pause {
    type Output = ();
    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("set_property"),
            Value::Str("pause"),
            Value::Bool(true),
        ]
    }
}

pub struct QueueClear;

impl Command<1> for QueueClear {
    type Output = ();

    fn cmd(&self) -> [Value<'_>; 1] {
        [Value::Str("playlist-clear")]
    }
}

pub struct LoadFile(pub PathBuf);

impl Command<3> for LoadFile {
    type Output = ();

    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("loadfile"),
            Value::Path(&self.0),
            Value::Str("append"), // TODO: don't hardcode param
        ]
    }
}

pub struct QueueMove {
    pub from: usize,
    pub to: usize,
}

impl Command<3> for QueueMove {
    type Output = ();

    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("playlist-move"),
            Value::Int(self.from as _),
            Value::Int(self.from as _),
        ]
    }
}

pub struct QueueRemove(pub usize);

impl Command<2> for QueueRemove {
    type Output = ();

    fn cmd(&self) -> [Value<'_>; 2] {
        [Value::Str("playlist-remove"), Value::Int(self.0 as _)]
    }
}

pub struct QueueLoad(pub PathBuf);

impl Command<3> for QueueLoad {
    type Output = ();

    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("loadlist"),
            Value::Path(&self.0),
            Value::Str("append"), // TODO: don't hardcode param
        ]
    }
}

pub struct QueueLoop(pub bool);

impl Command<3> for QueueLoop {
    type Output = ();

    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("set_property"),
            Value::Str("loop-playlist"),
            Value::Str(if self.0 { "inf" } else { "no" }),
        ]
    }
}

pub struct QueueShuffle;

impl Command<1> for QueueShuffle {
    type Output = ();

    fn cmd(&self) -> [Value<'_>; 1] {
        [Value::Str("playlist-shuffle")]
    }
}

macro_rules! get_prop_impl {
    ($($name:ident, $prop:expr => $o:ty);*$(;)?) => {
        $(
        pub struct $name;

        impl crate::socket::cmds::command::Command<2> for $name {
            type Output = $o;

            fn cmd(&self) -> [Value<'_>; 2] {
                [Value::Str("get_property"), Value::Str($prop)]
            }
        }
        )*
    }
}

get_prop_impl!(
    QueuePos, "playlist-pos" => usize;
    Queue, "playlist" => Vec<QueueItem>;
    Filename, "filename" => String;
    MediaTitle, "media-title" => String;
    ChapterMetadata, "chapter-metadata" => String;
    IsPaused, "pause" => bool;
    Volume, "volume" => f64;
    PercentPosition, "percent-pos" => f64;
    QueueSize, "playlist-count" => usize;
    QueueIsLooping, "loop-playlist" => bool;
);

#[derive(Deserialize)]
pub struct QueueItem {
    pub filename: String,
    #[serde(default)]
    pub current: bool,
    #[serde(default)]
    pub playing: bool,
    pub id: usize,
}

#[cfg(test)]
mod test {
    use super::*;
    use serde_json::{json, to_value};
    use std::f64::consts::PI;

    #[test]
    fn static_args() {
        let a = [
            Value::Str("str"),
            Value::Bool(true),
            Value::Float(PI),
            Value::Int(42),
        ];

        assert_eq!(to_value(a).unwrap(), json!(["str", true, PI, 42]))
    }
}
