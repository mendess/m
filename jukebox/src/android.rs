mod log;
use crate::{Ui, UiResult};
use jni::{
    errors::Result as JniResult,
    objects::{JClass, JString},
    JNIEnv,
};
use crate::i;
use parking_lot::{const_mutex, Condvar, Mutex};
use std::{fmt::Display, time::Duration};

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
        // let jvm = env.get_java_vm()?;
        std::thread::spawn(|| {
            crate::relay::user::run(addr, Duration::from_secs(5), &UI)
        });
        Ok(())
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
        UI.0.lock().room_name = Some(name);
        UI.1.notify_one();
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
        UI.0.lock().commands.push(command);
        UI.1.notify_one();
        Ok(())
    }
    do_or_throw!(env, send_command(&env, input))
}

static UI: (Mutex<AndroidUi>, Condvar) =
    (const_mutex(AndroidUi::new()), Condvar::new());

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

impl Ui for &(Mutex<AndroidUi>, Condvar) {
    fn room_name(&mut self) -> UiResult {
        println!("Attempting to get room name");
        let mut l = self.0.lock();
        while l.room_name.is_none() {
            println!("Waiting for room name");
            self.1.wait(&mut l)
        }
        println!("Got room name");
        Ok(l.room_name.as_ref().map(|s| s.clone()).unwrap())
    }

    fn command(&mut self) -> UiResult {
        println!("Attempting to get command");
        let mut l = self.0.lock();
        while l.commands.is_empty() {
            println!("Waiting for command");
            self.1.wait(&mut l)
        }
        println!("Got command");
        Ok(l.commands.remove(0))
    }

    fn inform<T: Display, E: Display>(&mut self, r: &Result<T, E>) {
        crate::print_result(r)
    }
}
