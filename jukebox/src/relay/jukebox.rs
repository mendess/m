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

pub fn run(port: u16) -> io::Result<()> {
    let mut socket = crate::connect_to_relay(port)?;
    writeln!(socket, "jukebox")?;
    let mut prompt = Prompt::default();
    loop {
        if prompt.p("Input room name:")? == 0 {
            return Ok(())
        }
        writeln!(socket, "{}", prompt)?;
        let mut b = [false as u8; 1];
        socket.read(&mut b)?;
        if b[0] == 1 {
            break;
        }
        crate::print_result(&Err("Name taken"));
    }
    println!("Room created");
    std::thread::spawn(|| local_client(prompt));
    let (reader, mut writer) = &mut (&socket, &socket);
    for r in Deserializer::from_reader(BufReader::new(reader)).into_iter::<Request>() {
        let r = match r {
            Ok(r) => r,
            Err(e) if e.is_eof() => break,
            Err(e) => return Err(e.into()),
        };
        let cmd = r.command().collect::<Vec<_>>();
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