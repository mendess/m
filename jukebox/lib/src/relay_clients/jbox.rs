use crate::{
    net::{
        reconnect::{Receiver, Reconnect, Sender},
        socket_channel::{SocketChannelReceive, SocketChannelSend},
    },
    prompt::{pretty_prompt, Prompt},
    relay::{
        rooms::{Request, Response},
        socket_server::Protocol,
    },
    try_prompt, RoomName,
};
use std::{
    borrow::Borrow, cell::RefCell, convert::Infallible, fmt::Write as _, io,
    net::ToSocketAddrs, process::Command, rc::Rc, time::Duration,
};

pub fn start_protocol<A: ToSocketAddrs>(
    addr: A,
    reconnect: Duration,
) -> io::Result<(
    Reconnect<impl Fn(&mut Receiver, &mut Sender) -> io::Result<()> + Clone>,
    Rc<RefCell<String>>,
)> {
    let room_name = Rc::new(RefCell::new(String::new()));
    let ref_room = Rc::clone(&room_name);
    let mut socket = Reconnect::connect(addr, reconnect, move |recv, send| {
        println!("Sending reconnect");
        send.send(Protocol::Reconnect)?;
        println!("Sending room name");
        send.send(&*ref_room)?;
        println!("Reading response");
        recv.recv()?;
        Ok(())
    })?;
    socket.send(Protocol::Jukebox)?;
    Ok((socket, room_name))
}

pub fn execute_loop<T>(
    socket: Reconnect<T>,
    prompt: Prompt,
) -> io::Result<Infallible>
where
    T: Fn(&mut Receiver, &mut Sender) -> io::Result<()> + Clone,
{
    std::thread::spawn(|| local_client(prompt));
    let (mut reader, mut writer) = socket.split()?;
    loop {
        let r = reader.recv::<Request>()?;
        let data = execute(&*r.cmd_line)?;
        writer.send(Response::new(r, data))?;
    }
}

pub fn with_room_name<A: ToSocketAddrs>(
    addr: A,
    reconnect: Duration,
    room_name: RoomName,
) -> io::Result<()> {
    let (mut socket, ref_room_name) = start_protocol(addr, reconnect)?;
    if {
        socket.send(&room_name)?;
        socket.recv()?
    } {
        *ref_room_name.borrow_mut() = room_name.name.clone();
        execute_loop(socket, pretty_prompt().with_room_name(room_name))
            .map(|_| ())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, "room name taken"))
    }
}

pub fn run<A: ToSocketAddrs>(addr: A, reconnect: Duration) -> io::Result<()> {
    let (mut socket, room_name) = start_protocol(addr, reconnect)?;
    let mut prompt = pretty_prompt();
    loop {
        let rn = try_prompt!(prompt.ask_room_name());
        if {
            socket.send(rn)?;
            socket.recv()?
        } {
            let _ = writeln!(room_name.borrow_mut(), "{}", prompt);
            break;
        }
        prompt.inform(&Result::<&str, _>::Err("Name taken"));
    }
    println!("Room created");
    execute_loop(socket, prompt).map(|_| ())
}

fn local_client(mut p: Prompt) -> io::Result<()> {
    let r = loop {
        let cmd = try_prompt!(p.command(), break);
        let data = execute(
            &cmd.split_whitespace()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>(),
        )?;
        p.inform(&data);
    };
    println!("Local prompt terminated, Ctrl+C to kill jukebox....");
    r
}

pub fn execute<I, S>(args: I) -> io::Result<Result<String, String>>
where
    I: IntoIterator<Item = S> + std::fmt::Debug,
    S: AsRef<std::ffi::OsStr>,
{
    println!("Executing command: {:?}", args);
    let o = Command::new("m").args(args).output()?;
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s += String::from_utf8_lossy(&o.stderr).borrow();
    if o.status.success() {
        Ok(Ok(s))
    } else {
        Ok(Err(s))
    }
}
