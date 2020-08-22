use crate::{reconnect::Reconnect as TcpStream, Ui, UiError};
use serde_json::Deserializer;
use std::{
    cell::RefCell,
    io::{self, Read, Write},
    net::ToSocketAddrs,
    time::Duration,
};

pub fn run<'p, A, P>(addr: A, reconnect: Duration, mut prompt: P) -> io::Result<()>
where
    A: ToSocketAddrs,
    P: Ui,
{
    let room_name = RefCell::new(String::new());
    let mut socket = TcpStream::connect(addr, reconnect, |s| {
        writeln!(s, "user")?;
        writeln!(s, "{}", room_name.borrow())?;
        s.read(&mut [0])?;
        Ok(())
    })?;
    writeln!(socket, "user")?;
    loop {
        let rn = match prompt.room_name() {
            Ok(rn) => rn,
            Err(UiError::Closed) => return Ok(()),
            Err(UiError::Io(e)) => return Err(e),
        };
        writeln!(socket, "{}", rn)?;
        let mut b = [false as u8; 1];
        socket.read(&mut b)?;
        if b[0] == 1 {
            let mut room_name = room_name.borrow_mut();
            room_name.clear();
            room_name.push_str(&rn);
            break;
        }
        prompt.inform(&Result::<&str, _>::Err("No such room"));
    }
    println!("Room joined");
    let (reader, mut writer) = socket.split()?;
    let mut responses =
        Deserializer::from_reader(reader).into_iter::<Result<String, String>>();
    loop {
        let cmd = match prompt.command() {
            Ok(cmd) => cmd,
            Err(UiError::Closed) => return Ok(()),
            Err(UiError::Io(e)) => return Err(e),
        };
        writeln!(writer, "{}", cmd)?;
        let r = match responses.next() {
            Some(r) => r,
            None => break,
        };
        let r = match r {
            Ok(r) => r,
            Err(e) => return Err(e.into()),
        };
        prompt.inform(&r);
    }
    Ok(())
}
