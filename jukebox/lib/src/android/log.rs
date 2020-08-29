use jni::{errors::Result as JniResult, objects::JValue, JNIEnv};

#[allow(dead_code)]
pub struct Assert;
#[allow(dead_code)]
pub struct Debug;
#[allow(dead_code)]
pub struct Error;
#[allow(dead_code)]
pub struct Info;
#[allow(dead_code)]
pub struct Verbose;
#[allow(dead_code)]
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

pub fn log<'e, 'jni: 'e, K>(env: &'e JNIEnv<'jni>, msg: &str) -> JniResult<()>
where
    K: LogKind,
{
    let class = env.find_class("android/util/Log")?;
    let msg = env.new_string(msg)?;
    let tag = {
        env.get_static_field(
            env.find_class("xyz/mendess/jukebox/JukeboxLib")?,
            "LOG_TAG",
            "Ljava/lang/String;"
        )?
    };
    env.call_static_method(
        class,
        K::name(),
        "(Ljava/lang/String;Ljava/lang/String;)I",
        &[tag, JValue::Object(msg.into())],
    )?;
    Ok(())
}

#[macro_export]
macro_rules! _base {
    ($t:ty, $env:expr, $format:expr, $($args:expr),*$(,)?) => {
        match $crate::android::log::log::<$t>(&$env, &format!($format, $($args),*)) {
            Ok(s) => s,
            Err(e) => {
                $env.throw_new("java/lang/Exception", e.to_string()).unwrap();
            },
        }
    }
}

#[macro_export]
macro_rules! i {
    ($env:expr, $format:expr) => { i!($env, $format,) };
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::_base!($crate::android::log::Info, $env, $format, $($args),*);
    }
}

#[macro_export]
macro_rules! d {
    ($env:expr, $format:expr) => { d!($env, $format,) };
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::_base!($crate::android::log::Debug, $env, $format, $($args),*);
    }
}

#[macro_export]
macro_rules! w {
    ($env:expr, $format:expr) => { w!($env, $format,) };
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::_base!($crate::android::log::Warn, $env, $format, $($args),*);
    }
}

#[macro_export]
macro_rules! wtf {
    ($env:expr, $format:expr) => { wtf!($env, $format,) };
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::_base!($crate::android::log::Assert, $env, $format, $($args),*);
    }
}

#[macro_export]
macro_rules! v {
    ($env:expr, $format:expr) => { v!($env, $format,) };
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::_base!($crate::android::log::Verbose, $env, $format, $($args),*);
    }
}

#[macro_export]
macro_rules! e {
    ($env:expr, $format:expr) => { e!($env, $format,) };
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $crate::_base!($crate::android::log::Error, $env, $format, $($args),*);
    }
}
