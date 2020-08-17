use crate::prompt::Prompt;
use serde_json::Deserializer;
use std::io::{self, Read, Write};

pub fn run(port: u16) -> io::Result<()> {
    let mut socket = crate::connect_to_relay(port)?;
    writeln!(socket, "user")?;
    let mut prompt = Prompt::default();
    loop {
        if prompt.p("Input room name:")? == 0 {
            return Ok(());
        }
        writeln!(socket, "{}", prompt)?;
        let mut b = [false as u8; 1];
        socket.read(&mut b)?;
        if b[0] == 1 {
            break;
        }
        crate::print_result(&Err("No such room"));
    }
    println!("Room joined");
    let (reader, writer) = &mut (&socket, &socket);
    shell(prompt, reader, writer)?;
    Ok(())
}

pub fn shell<R, W>(
    mut prompt: Prompt,
    reader: R,
    mut writer: W,
) -> io::Result<()>
where
    R: Read,
    W: Write,
{
    let mut responses =
        Deserializer::from_reader(reader).into_iter::<Result<String, String>>();
    while prompt.p("🎵>")? > 0 {
        writeln!(writer, "{}", prompt)?;
        let r = match responses.next() {
            Some(r) => r,
            None => break,
        };
        let r = match r {
            Ok(r) => r,
            Err(e) if e.is_eof() => break,
            Err(e) => return Err(e.into()),
        };
        match r {
            Ok(s) => println!("{}", s),
            Err(s) => println!("Error\n{}", s),
        }
    }

    Ok(())
}
