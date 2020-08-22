use jni::{errors::Result as JniResult, objects::JValue, JNIEnv};

pub struct Assert;
pub struct Debug;
pub struct Error;
pub struct Info;
pub struct Verbose;
pub struct Warn;

pub trait LogKind {
    fn name() -> &'static str;
}

impl LogKind for Assert {
    fn name() -> &'static str {
        "wtf"
    }
}

impl LogKind for Debug {
    fn name() -> &'static str {
        "d"
    }
}

impl LogKind for Error {
    fn name() -> &'static str {
        "e"
    }
}

impl LogKind for Info {
    fn name() -> &'static str {
        "i"
    }
}

impl LogKind for Verbose {
    fn name() -> &'static str {
        "v"
    }
}

impl LogKind for Warn {
    fn name() -> &'static str {
        "w"
    }
}

pub fn log<K: LogKind>(env: &JNIEnv, msg: &str) -> JniResult<()> {
    let class = env.find_class("android/util/Log")?;
    let msg = env.new_string(msg)?;
    env.call_static_method(
        class,
        K::name(),
        "(Ljava/lang/String)V",
        &[JValue::Object(msg.into())],
    )?;
    Ok(())
}

#[macro_export]
macro_rules! i {
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::android::log::log::<$crate::android::log::Info>(&$env, &format!($format, $($args),*)).unwrap()
    }
}

#[macro_export]
macro_rules! d {
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::android::log::log::<$crate::android::log::Debug>(&$env, &format!($format, $($args),*)).unwrap()
    }
}

#[macro_export]
macro_rules! w {
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::android::log::log::<$crate::android::log::Warn>(&$env, &format!($format, $($args),*)).unwrap()
    }
}

#[macro_export]
macro_rules! wtf {
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::android::log::log::<$crate::android::log::Assert>(&$env, &format!($format, $($args),*)).unwrap()
    }
}

#[macro_export]
macro_rules! v {
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::android::log::log::<$crate::android::log::Verbose>(&$env, &format!($format, $($args),*)).unwrap()
    }
}

#[macro_export]
macro_rules! e {
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::android::log::log::<$crate::android::log::Error>(&$env, &format!($format, $($args),*)).unwrap()
    }
}
