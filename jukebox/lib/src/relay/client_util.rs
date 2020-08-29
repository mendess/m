use std::io::{self, Read, Write};

pub fn attempt_room_name<S: Read + Write>(
    mut socket: S,
    s: &str,
) -> io::Result<bool> {
    writeln!(socket, "{}", s)?;
    let mut b = [false as u8; 1];
    socket.read(&mut b)?;
    Ok(b[0] == 1)
}
