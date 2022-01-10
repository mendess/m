use serde::{
    de::{Unexpected, Visitor},
    Deserialize,
};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Hash)]
pub enum LoopStatus {
    Inf,
    Force,
    No,
    N(u64),
}

struct LoopStatusVisitor;

impl<'de> Visitor<'de> for LoopStatusVisitor {
    type Value = LoopStatus;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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
