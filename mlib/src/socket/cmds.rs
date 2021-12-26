use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::queue::Item;

use self::command::{Compute, Execute};

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

        impl crate::socket::cmds::command::Execute<2> for $name {
            fn cmd(&self) -> [Value<'_>; 2] {
                [Value::Str("get_property"), Value::Str($prop)]
            }
        }

        impl crate::socket::cmds::command::Compute<2> for $name {
            type Output = $o;
        }
        )*
    }
}

get_prop_impl!(
    ChapterMetadata, "chapter-metadata" => String;
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

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
pub enum LoopStatus {
    Inf,
    Force,
    No,
    N(u64),
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

mod loop_status {
    use super::LoopStatus;
    use serde::{
        de::{Unexpected, Visitor},
        Deserialize,
    };

    struct LoopStatusVisitor;

    impl<'de> Visitor<'de> for LoopStatusVisitor {
        type Value = LoopStatus;

        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str(r#""inf", "force", "no" or a positive integer"#)
        }

        fn visit_i8<E>(self, v: i8) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            u64::try_from(v)
                .map(LoopStatus::N)
                .map_err(|_| E::invalid_value(Unexpected::Signed(v.into()), &self))
        }

        fn visit_i16<E>(self, v: i16) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            u64::try_from(v)
                .map(LoopStatus::N)
                .map_err(|_| E::invalid_value(Unexpected::Signed(v.into()), &self))
        }

        fn visit_i32<E>(self, v: i32) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            u64::try_from(v)
                .map(LoopStatus::N)
                .map_err(|_| E::invalid_type(Unexpected::Signed(v.into()), &self))
        }

        fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            u64::try_from(v)
                .map(LoopStatus::N)
                .map_err(|_| E::invalid_type(Unexpected::Signed(v), &self))
        }

        fn visit_u8<E>(self, v: u8) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(LoopStatus::N(v.into()))
        }

        fn visit_u16<E>(self, v: u16) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(LoopStatus::N(v.into()))
        }

        fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(LoopStatus::N(v.into()))
        }

        fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            Ok(LoopStatus::N(v))
        }

        fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            match v {
                "inf" => Ok(LoopStatus::Inf),
                "force" => Ok(LoopStatus::Force),
                "no" => Ok(LoopStatus::No),
                _ => Err(E::unknown_variant(v, &["inf", "force", "no"])),
            }
        }

        fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            if v {
                Err(E::invalid_value(Unexpected::Bool(v), &self))
            } else {
                Ok(LoopStatus::No)
            }
        }
    }

    impl<'de> Deserialize<'de> for LoopStatus {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            deserializer.deserialize_any(LoopStatusVisitor)
        }
    }
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
