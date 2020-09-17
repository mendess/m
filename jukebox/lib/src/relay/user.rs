use crate::{
    reconnect::Reconnect as TcpStream, socket_channel, Ui, UiError,
    relay::Protocol,
};
use std::{
    cell::RefCell,
    io::{self, BufReader, Read, Write},
    net::ToSocketAddrs,
    time::Duration,
};

pub fn run<'p, A, P>(
    addr: A,
    reconnect: Duration,
    mut prompt: P,
) -> io::Result<()>
where
    A: ToSocketAddrs,
    P: Ui,
{
    let room_name = RefCell::new(String::new());
    prompt.inform("connecting to socket");
    let socket = TcpStream::connect(addr, reconnect, |s| {
        writeln!(s, "user")?;
        writeln!(s, "{}", room_name.borrow())?;
        s.read(&mut [0])?;
        Ok(())
    })?;
    let (reader, writer) = socket.split()?;
    let (mut receiver, mut sender) =
        socket_channel::make(BufReader::new(reader), writer);
    prompt.inform("setting client type to user");
    sender.send(Protocol::User)?;
    loop {
        let rn = match prompt.room_name() {
            Ok(rn) => rn,
            Err(UiError::Closed) => return Ok(()),
            Err(UiError::Io(e)) => return Err(e),
        };
        sender.send(&rn)?;
        if receiver.recv()? {
            let mut room_name = room_name.borrow_mut();
            room_name.clear();
            room_name.push_str(&rn);
            break;
        }
        prompt.inform(&Result::<&str, _>::Err("No such room"));
    }
    prompt.inform(&Result::<_, &str>::Ok("Room joined"));
    let mut s = String::new();
    loop {
        let cmd = match prompt.command() {
            Ok(cmd) => cmd,
            Err(UiError::Closed) => return Ok(()),
            Err(UiError::Io(e)) => return Err(e),
        };
        sender.send::<String>(cmd)?;
        let r = receiver.recv_with_buf::<Result<String, String>>(&mut s)?;
        prompt.inform(&r);
    }
}
