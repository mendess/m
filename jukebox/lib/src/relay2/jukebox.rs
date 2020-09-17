use crate::reconnect::Reconnect as TcpStream;
use crate::{
    prompt::Prompt,
    relay::{client_util::attempt_room_name, Request, Response},
};
use serde_json::Deserializer;
use std::{
    borrow::Borrow,
    cell::RefCell,
    fmt::Write as _,
    io::{self, BufReader, Read, Write},
    net::ToSocketAddrs,
    process::Command,
    time::Duration,
    rc::Rc,
};

pub fn execute(args: &[&str]) -> io::Result<Result<String, String>> {
    println!("Executing command: {:?}", args);
    let o = Command::new("m").args(args).output()?;
    // let mut s: String = String::from_utf8_lossy(&o.stdout).to_owned();
    let mut s = String::from_utf8_lossy(&o.stdout).to_string();
    s += String::from_utf8_lossy(&o.stderr).borrow();
    if o.status.success() {
        Ok(Ok(s))
    } else {
        Ok(Err(s))
    }
}

pub fn start_protocol<A: ToSocketAddrs>(
    addr: A,
    reconnect: Duration,
) -> io::Result<(
    TcpStream<impl Fn(&mut std::net::TcpStream) -> io::Result<()> + Clone>,
    Rc<RefCell<String>>,
)> {
    let room_name = Rc::new(RefCell::new(String::new()));
    let ref_room = Rc::clone(&room_name);
    let mut socket = TcpStream::connect(addr, reconnect, move |s| {
        println!("Sending reconnect");
        writeln!(s, "reconnect")?;
        println!("Sending room name");
        writeln!(s, "{}", (&*ref_room).borrow())?;
        println!("Reading response");
        s.read(&mut [0])?;
        Ok(())
    })?;
    writeln!(socket, "jukebox")?;
    Ok((socket, room_name))
}

pub fn run<A: ToSocketAddrs>(addr: A, reconnect: Duration) -> io::Result<()> {
    let (mut socket, room_name) = start_protocol(addr, reconnect)?;
    let mut prompt = Prompt::default();
    loop {
        if prompt.p("Input room name:")? == 0 {
            return Ok(());
        }
        if attempt_room_name(&mut socket, prompt.buf())? {
            let _ = writeln!(room_name.borrow_mut(), "{}", prompt);
            break;
        }
        crate::print_result(&Result::<&str, _>::Err("Name taken"));
    }
    println!("Room created");
    std::thread::spawn(|| local_client(prompt));
    execute_loop(socket)
}

pub fn execute_loop<T>(socket: TcpStream<T>) -> io::Result<()>
where
    T: Fn(&mut std::net::TcpStream) -> io::Result<()> + Clone,
{
    let (reader, mut writer) = socket.split()?;
    for r in
        Deserializer::from_reader(BufReader::new(reader)).into_iter::<Request>()
    {
        let r = match r {
            Ok(r) => r,
            Err(e) if e.is_eof() => break,
            Err(e) => return Err(e.into()),
        };
        let cmd = r.command();
        let data = execute(&cmd)?;
        serde_json::to_writer(&mut writer, &Response::new(r, data))?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}

fn local_client(mut p: Prompt) -> io::Result<()> {
    while p.p("🎵>")? > 0 {
        let data = execute(
            &p.buf()
                .split_whitespace()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>(),
        )?;
        crate::print_result(&data);
    }
    println!("Local prompt terminated, Ctrl+C to kill jukebox....");
    Ok(())
}
