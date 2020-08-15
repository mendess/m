use crate::{
    prompt::Prompt,
    relay::{Request, Response},
};
use serde_json::Deserializer;
use std::{
    borrow::Borrow,
    io::{self, BufReader, Read, Write},
    process::Command,
};

pub fn run(port: u16) -> io::Result<()> {
    let mut socket = crate::connect_to_relay(port)?;
    writeln!(socket, "jukebox")?;
    let mut prompt = Prompt::default();
    while prompt.p("Input room name:")? > 0 {
        writeln!(socket, "{}", prompt)?;
        let mut b = [false as u8; 1];
        socket.read(&mut b)?;
        if b[0] == 1 {
            break;
        }
        println!("\x1b[1;31mError:\x1b[0m Name taken");
    }
    println!("Room created");
    drop(prompt);
    let (reader, mut writer) = &mut (&socket, &socket);
    for r in Deserializer::from_reader(BufReader::new(reader)).into_iter::<Request>() {
        let r = match r {
            Ok(r) => r,
            Err(e) if e.is_eof() => break,
            Err(e) => return Err(e.into()),
        };
        let cmd = r.command().collect::<Vec<_>>();
        println!("Executing command: {:?}", cmd);
        let o = Command::new("m").args(&cmd).output()?;
        // let mut s: String = String::from_utf8_lossy(&o.stdout).to_owned();
        let mut s = String::from_utf8_lossy(&o.stdout).to_string();
        s += String::from_utf8_lossy(&o.stderr).borrow();
        let data = if o.status.success() { Ok(s) } else { Err(s) };
        serde_json::to_writer(&mut writer, &Response::new(r, data))?;
        writer.write_all(b"\n")?;
    }
    Ok(())
}
