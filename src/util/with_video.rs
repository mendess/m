use std::{env, ffi::OsStr};

pub fn with_video_env() -> bool {
    env::var_os("WITH_VIDEO").as_deref() == Some(OsStr::new("1"))
}
