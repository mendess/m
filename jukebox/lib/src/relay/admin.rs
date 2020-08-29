use crate::prompt::Prompt;
use serde_json::Deserializer;
use std::{net::{ToSocketAddrs, TcpStream},io::{self, Write}};

pub fn run<A: ToSocketAddrs>(addr: A) -> io::Result<()> {
    let mut socket = TcpStream::connect(addr)?;
    writeln!(socket, "admin")?;
    let (reader, writer) = &mut (&socket, &socket);
    let mut responses =
        Deserializer::from_reader(reader).into_iter::<Result<String, String>>();
    let mut prompt = Prompt::default();
    while prompt.p("root>")? > 0 {
        writeln!(writer, "{}", prompt)?;
        let r = match responses.next() {
            Some(r) => r?,
            None => break,
        };
        match r {
            Ok(s) => println!("{}", s),
            Err(e) => println!("Error\n{}", e),
        }
    }
    Ok(())
}
