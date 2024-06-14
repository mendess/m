use core::fmt;
use std::io;

use serde::{Deserialize, Serialize};

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Io: {0}")]
    Io(#[from] io::Error),
    #[error("Other: {0}")]
    Mpv(#[from] MpvError),
}

#[derive(thiserror::Error, Debug, Clone, Serialize, Deserialize)]
pub enum MpvError {
    #[error("mpv error. code: {0}")]
    Raw(MpvErrorCode),
    #[cfg(feature = "player")]
    #[error("load files. index: {0} code: {1}")]
    Loadfiles(usize, libmpv::MpvError),
    #[error("No player running")]
    NoMpvInstance,
    #[error("invalid utf8")]
    InvalidUtf8,
    #[error("invalid data returned from mpv. Expected {expected} but got '{got}': {error:?}")]
    InvalidData {
        expected: String,
        got: String,
        error: String,
    },
    #[error("failed to execute command because {reason}")]
    FailedToExecute { reason: String },
}

#[cfg(feature = "player")]
impl From<libmpv::Error> for MpvError {
    fn from(e: libmpv::Error) -> Self {
        use libmpv::Error::*;
        match e {
            Loadfiles { index, error } => Self::Loadfiles(
                index,
                match &*error {
                    Raw(e) => *e,
                    _ => panic!("unexpected error: {error:?}"),
                },
            ),
            InvalidUtf8 => MpvError::InvalidUtf8,
            Raw(e) => Self::Raw(e.into()),
            VersionMismatch { linked, loaded } => {
                panic!("version mismatch. linked: {linked}, loaded: {loaded}")
            }
            Null => panic!("null byte found"),
        }
    }
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)]
pub enum MpvErrorCode {
    Success = 0,
    EventQueueFull = -1,
    Nomem = -2,
    Uninitialized = -3,
    InvalidParameter = -4,
    OptionNotFound = -5,
    OptionFormat = -6,
    OptionError = -7,
    PropertyNotFound = -8,
    PropertyFormat = -9,
    PropertyUnavailable = -10,
    PropertyError = -11,
    Command = -12,
    LoadingFailed = -13,
    AoInitFailed = -14,
    VoInitFailed = -15,
    NothingToPlay = -16,
    UnknownFormat = -17,
    Unsupported = -18,
    NotImplemented = -19,
    Generic = -20,
    Unknown = i32::MIN,
}

#[cfg(feature = "player")]
impl From<libmpv::MpvError> for MpvErrorCode {
    fn from(e: libmpv::MpvError) -> Self {
        if (-20..=0i32).contains(&e) {
            unsafe { std::mem::transmute::<i32, MpvErrorCode>(e) }
        } else {
            Self::Unknown
        }
    }
}

impl fmt::Display for MpvErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        <Self as fmt::Debug>::fmt(self, f)
    }
}

pub type MpvResult<T> = ::std::result::Result<T, MpvError>;
