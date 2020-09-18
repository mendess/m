use serde::{de::DeserializeOwned, Serialize};
use serde_json::error::Category;
use std::io::{BufRead, Write};
use tokio::io::{self, AsyncBufReadExt, AsyncWriteExt};

#[derive(Debug)]
pub struct Receiver<S> {
    socket: S,
    message_buffer: Vec<String>,
}

#[derive(Debug)]
pub struct Sender<S> {
    socket: S,
}

pub fn make<'s, R, W>(read_half: R, write_half: W) -> (Receiver<R>, Sender<W>) {
    (
        Receiver {
            socket: read_half,
            message_buffer: Default::default(),
        },
        Sender { socket: write_half },
    )
}

impl<W: AsyncWriteExt + Unpin> Sender<W> {
    pub async fn asend<M: Serialize>(&mut self, m: M) -> io::Result<()> {
        let msg = serde_json::to_string(&m)?;
        self.socket.write_all(msg.as_bytes()).await?;
        self.socket.write_all(b"\n").await?;
        Ok(())
    }
}

impl<W: Write> SocketChannelSend for Sender<W> {
    fn send<M: Serialize>(&mut self, m: M) -> io::Result<()> {
        serde_json::to_writer(&mut self.socket, &m)?;
        writeln!(self.socket)
    }
}

impl<R: AsyncBufReadExt + Unpin> Receiver<R> {
    pub async fn arecv<M: DeserializeOwned>(&mut self) -> io::Result<M> {
        self.arecv_with_buf(&mut String::new()).await
    }

    pub async fn arecv_with_buf<M: DeserializeOwned>(
        &mut self,
        buf: &mut String,
    ) -> io::Result<M> {
        for (i, m) in self.message_buffer.iter().enumerate() {
            if let Ok(m) = serde_json::from_str(&m) {
                self.message_buffer.remove(i);
                return Ok(m);
            }
        }
        loop {
            buf.clear();
            self.socket.read_line(buf).await?;
            buf.pop();
            match serde_json::from_str(&buf) {
                Ok(m) => return Ok(m),
                Err(e)
                    if matches!(e.classify(), Category::Io | Category::Eof) =>
                {
                    return Err(e.into())
                }
                _ => self.message_buffer.push(buf.clone()),
            }
        }
    }
}

impl<R: BufRead> SocketChannelReceive for Receiver<R> {
    fn recv_with_buf<M: DeserializeOwned>(
        &mut self,
        buf: &mut String,
    ) -> io::Result<M> {
        for (i, m) in self.message_buffer.iter().enumerate() {
            if let Ok(m) = serde_json::from_str(&m) {
                self.message_buffer.remove(i);
                return Ok(m);
            }
        }
        loop {
            buf.clear();
            self.socket.read_line(buf)?;
            buf.pop();
            match serde_json::from_str(&buf) {
                Ok(m) => return Ok(m),
                Err(e)
                    if matches!(e.classify(), Category::Io | Category::Eof) =>
                {
                    return Err(e.into())
                }
                _ => self.message_buffer.push(buf.clone()),
            }
        }
    }
}

pub trait SocketChannelReceive {
    fn recv<M: DeserializeOwned>(&mut self) -> io::Result<M> {
        self.recv_with_buf(&mut String::new())
    }

    fn recv_with_buf<M: DeserializeOwned>(
        &mut self,
        buf: &mut String,
    ) -> io::Result<M>;
}

pub trait SocketChannelSend {
    fn send<M: Serialize>(&mut self, m: M) -> io::Result<()>;
}
