mod log;
use crate::{i, Information, Ui, UiResult};
use jni::{
    errors::{Error, ErrorKind, Result as JniResult},
    objects::{JClass, JString},
    JNIEnv, JavaVM,
};
use once_cell::sync::OnceCell;
use parking_lot::{const_mutex, Condvar, Mutex};
use std::time::Duration;

macro_rules! do_or_throw {
    ($env:expr, $do:expr) => {
        do_or_throw!($env, $do, "java/lang/Exception");
    };
    ($env:expr, $do:expr, $e:expr) => {
        match $do {
            Ok(s) => s,
            Err(e) => return $env.throw_new($e, e.to_string()).unwrap(),
        }
    };
}

#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "C" fn Java_xyz_mendess_jukebox_JukeboxLib_startUserThread(
    env: JNIEnv,
    _: JClass,
    input: JString,
) {
    fn start_user_thread(env: &JNIEnv, input: JString) -> JniResult<()> {
        let addr: String = env.get_string(input)?.into();
        UI.jvm.set(env.get_java_vm()?).map_err(|_| {
            Error::from(ErrorKind::Msg(
                "Only one user thread can be started".into(),
            ))
        })?;
        i!(env, "Starting client");
        crate::relay::user::run(addr, Duration::from_secs(5), &UI)
            .map_err(|e| Error::from_kind(ErrorKind::Msg(e.to_string())))
    }
    do_or_throw!(env, start_user_thread(&env, input))
}

#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "C" fn Java_xyz_mendess_jukebox_JukeboxLib_setRoomName(
    env: JNIEnv,
    _: JClass,
    input: JString,
) {
    fn set_room_name(env: &JNIEnv, input: JString) -> JniResult<()> {
        let name = env.get_string(input)?.into();
        i!(env, "Setting room name '{}'", name);
        UI.android_ui.lock().room_name = Some(name);
        UI.condvar.notify_one();
        Ok(())
    }
    do_or_throw!(env, set_room_name(&env, input))
}

#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "C" fn Java_xyz_mendess_jukebox_JukeboxLib_sendCommand(
    env: JNIEnv,
    _: JClass,
    input: JString,
) {
    fn send_command(env: &JNIEnv, input: JString) -> JniResult<()> {
        let command = env.get_string(input)?.into();
        i!(env, "Sending command '{}'", command);
        UI.android_ui.lock().commands.push(command);
        UI.condvar.notify_one();
        Ok(())
    }
    do_or_throw!(env, send_command(&env, input))
}

static UI: GlobalUi = GlobalUi::new();

struct AndroidUi {
    room_name: Option<String>,
    commands: Vec<String>,
}

impl AndroidUi {
    const fn new() -> AndroidUi {
        Self {
            room_name: None,
            commands: Vec::new(),
        }
    }
}

struct GlobalUi {
    android_ui: Mutex<AndroidUi>,
    condvar: Condvar,
    jvm: OnceCell<JavaVM>,
}

impl GlobalUi {
    const fn new() -> Self {
        Self {
            android_ui: const_mutex(AndroidUi::new()),
            condvar: Condvar::new(),
            jvm: OnceCell::new(),
        }
    }
}

macro_rules! log {
    ($env:expr, $format:expr) => { log!($env, $format,) };
    ($env:expr, $format:expr, $($args:expr),*$(,)?) => {
        $env.as_ref().map(|e| i!(e, $format, $($args),*))
    }
}

impl Ui for &GlobalUi {
    fn room_name(&mut self) -> UiResult {
        let env = self.jvm.get().map(|j| j.get_env().unwrap());
        let mut l = self.android_ui.lock();
        log!(env, "Attempting to get room name");
        while l.room_name.is_none() {
            log!(env, "Waiting for room name");
            self.condvar.wait(&mut l)
        }
        log!(env, "Got room name");
        Ok(l.room_name.as_ref().map(|s| s.clone()).unwrap())
    }

    fn command(&mut self) -> UiResult {
        let env = self.jvm.get().map(|j| j.get_env().unwrap());
        let mut l = self.android_ui.lock();
        log!(env, "Attempting to get command");
        while l.commands.is_empty() {
            log!(env, "Waiting for command");
            self.condvar.wait(&mut l)
        }
        log!(env, "Got command");
        Ok(l.commands.remove(0))
    }

    fn inform<I: Information>(&mut self, r: I) {
        let env = self.jvm.get().map(|j| j.get_env().unwrap());
        r.info(|d| {
            log!(env, "Inform: {}", d);
        });
    }
}
