use crate::prompt::Prompt;
use serde_json::Deserializer;
use std::io::{self, Write};

pub fn run(port: u16) -> io::Result<()> {
    let mut socket = crate::connect_to_relay(port)?;
    writeln!(socket, "admin")?;
    let (reader, writer) = &mut (&socket, &socket);
    let mut responses = Deserializer::from_reader(reader).into_iter::<Result<String, String>>();
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
