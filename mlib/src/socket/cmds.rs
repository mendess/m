mod loop_status;

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::item::Item;

use self::command::{Compute, Execute};

pub use loop_status::LoopStatus;

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

    pub trait Execute<const N: usize> {
        fn cmd(&self) -> [Value<'_>; N];
    }

    pub trait Compute<const N: usize>: Execute<N> {
        type Output: DeserializeOwned;
    }

    pub trait Property {
        const NAME: &'static str;
        type Output: DeserializeOwned;
    }

    impl<T> Execute<2> for T
    where
        T: Property,
    {
        fn cmd(&self) -> [Value<'_>; 2] {
            [Value::Str("get_property"), Value::Str(T::NAME)]
        }
    }

    impl<T> Compute<2> for T
    where
        T: Property,
    {
        type Output = T::Output;
    }
}

pub struct Pause;

impl Execute<3> for Pause {
    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("set_property"),
            Value::Str("pause"),
            Value::Bool(true),
        ]
    }
}

pub struct QueueClear;

impl Execute<1> for QueueClear {
    fn cmd(&self) -> [Value<'_>; 1] {
        [Value::Str("playlist-clear")]
    }
}

pub struct LoadFile<'f>(pub &'f Item);

impl Execute<3> for LoadFile<'_> {
    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("loadfile"),
            match &self.0 {
                Item::Link(l) => Value::Str(l.as_str()),
                Item::File(f) => Value::Path(f),
                Item::Search(s) => Value::Str(s.as_str()),
            },
            Value::Str("append"), // TODO: don't hardcode param
        ]
    }
}

pub struct LoadList<'f>(pub &'f Path);

impl Execute<3> for LoadList<'_> {
    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("loadlist"),
            Value::Path(self.0),
            Value::Str("append"),
        ]
    }
}

pub struct QueueMove {
    pub from: usize,
    pub to: usize,
}

impl Execute<3> for QueueMove {
    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("playlist-move"),
            Value::Int(self.from as _),
            Value::Int(self.to as _),
        ]
    }
}

pub struct QueueRemove(pub usize);

impl Execute<2> for QueueRemove {
    fn cmd(&self) -> [Value<'_>; 2] {
        [Value::Str("playlist-remove"), Value::Int(self.0 as _)]
    }
}

pub struct QueueLoad(pub PathBuf);

impl Execute<3> for QueueLoad {
    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("loadlist"),
            Value::Path(&self.0),
            Value::Str("append"), // TODO: don't hardcode param
        ]
    }
}

pub struct QueueLoop(pub bool);

impl Execute<3> for QueueLoop {
    fn cmd(&self) -> [Value<'_>; 3] {
        [
            Value::Str("set_property"),
            Value::Str("loop-playlist"),
            Value::Str(if self.0 { "inf" } else { "no" }),
        ]
    }
}

pub struct QueueShuffle;

impl Execute<1> for QueueShuffle {
    fn cmd(&self) -> [Value<'_>; 1] {
        [Value::Str("playlist-shuffle")]
    }
}

macro_rules! get_prop_impl {
    ($($name:ident, $prop:expr => $o:ty);*$(;)?) => {
        $(
        pub struct $name;

        impl $crate::socket::cmds::command::Property for $name {
            const NAME: &'static str = $prop;
            type Output = $o;
        }
        )*
    }
}

get_prop_impl!(
    ChapterMetadata, "chapter-metadata" => Metadata;
    Filename, "filename" => String;
    IsPaused, "pause" => bool;
    MediaTitle, "media-title" => String;
    PercentPosition, "percent-pos" => f64;
    Queue, "playlist" => Vec<QueueItem>;
    QueueFilename, "filename" => String;
    QueueIsLooping, "loop-playlist" => LoopStatus;
    QueuePos, "playlist-pos" => usize;
    QueueSize, "playlist-count" => usize;
    Volume, "volume" => f64;
);

#[derive(Deserialize, Debug)]
pub struct Metadata {
    pub title: String,
}

#[derive(Deserialize, Debug)]
pub struct QueueItem {
    pub filename: String,
    #[serde(default, flatten)]
    pub status: Option<QueueItemStatus>,
    pub id: usize,
}

#[derive(Deserialize, Debug)]
pub struct QueueItemStatus {
    pub current: bool,
    pub playing: bool,
}

pub struct QueueNFilename(pub usize);

impl Execute<2> for QueueNFilename {
    fn cmd(&self) -> [Value<'_>; 2] {
        [
            Value::Str("get_property"),
            Value::String(format!("playlist/{}/filename", self.0)),
        ]
    }
}

impl Compute<2> for QueueNFilename {
    type Output = String;
}

pub struct QueueN(pub usize);

impl Execute<2> for QueueN {
    fn cmd(&self) -> [Value<'_>; 2] {
        [
            Value::Str("get_property"),
            Value::String(format!("playlist/{}", self.0)),
        ]
    }
}

impl Compute<2> for QueueN {
    type Output = QueueItem;
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

    #[test]
    fn loop_status_inf() {
        assert_eq!(LoopStatus::Inf, serde_json::from_str(r#""inf""#).unwrap())
    }

    #[test]
    fn loop_status_force() {
        assert_eq!(
            LoopStatus::Force,
            serde_json::from_str(r#""force""#).unwrap()
        )
    }

    #[test]
    fn loop_status_no() {
        assert_eq!(LoopStatus::No, serde_json::from_str(r#""no""#).unwrap())
    }

    #[test]
    fn loop_status_n() {
        assert_eq!(LoopStatus::N(42), serde_json::from_str("42").unwrap());
    }
}
