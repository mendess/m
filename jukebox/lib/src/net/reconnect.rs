use socket2::{Domain, Protocol, Socket, Type};
use crate::net::socket_channel::{self, Receiver, Sender};
use std::{
    fmt,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::Arc,
    thread::sleep,
    time::Duration,
    cell::RefCell,
};

pub const KEEP_ALIVE: Duration = Duration::from_secs(10);

pub struct Reconnect<R> {
    receiver: Arc<RefCell<Receiver<TcpStream>>>,
    sender: Arc<<RefCell<Sender<TcpStream>>>,
    addr: SocketAddr,
    timeout: Duration,
    protocol: R,
}

fn configure_socket(addr: SocketAddr) -> io::Result<(Receiver<TcpStream>, Sender<TcpStream>)> {
    let s = Socket::new(Domain::ipv4(), Type::stream(), Some(Protocol::tcp()))?;
    s.set_keepalive(Some(KEEP_ALIVE))?;
    s.connect(&addr.into())?;
    let tcp = s.into_tcp_stream();
    let (recv, send) = socket_channel::make(tcp.try_clone()?, tcp);
    Ok((recv, send))
}

impl<R> Reconnect<R>
where
    R: Fn(&mut TcpStream) -> io::Result<()>,
{
    pub fn connect<A: ToSocketAddrs>(
        addr: A,
        timeout: Duration,
        protocol: R,
    ) -> io::Result<Self> {
        let addr = addr.to_socket_addrs()?.next().ok_or(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid socket address",
        ))?;
        let (recv, send) = configure_socket(addr)?;
        Ok(Self {
            addr,
            receiver: Arc::new(RefCell::new(recv)),
            sender: Arc::new(RefCell::new(send)),
            timeout,
            protocol,
        })
    }
}

impl<R> Reconnect<R>
where
    R: Fn(&mut TcpStream) -> io::Result<()> + Clone,
{
    pub fn split(self) -> io::Result<(Self, Self)> {
        let receiver = Arc::clone(&self.receiver);
        let sender = Arc::clone(&self.sender);
        let protocol = self.protocol.clone();
        Ok((
            Reconnect {
                receiver,
                sender,
                protocol,
                ..self
            },
            self,
        ))
    }
}

impl<R> Reconnect<R>
where
    R: Fn(&mut Receiver<TcpStream>, &mut Sender<TcpStream>) -> io::Result<()>,
{
    fn do_reconnect<F, T: std::fmt::Debug>(&mut self, mut f: F) -> io::Result<T>
    where
        F: FnMut(&mut Receiver<TcpStream>, &mut Sender<TcpStream>) -> io::Result<T>,
    {
        loop {
            match { f(&mut *self.receiver, &mut *self.sender) } {
                Err(e) if e.kind() == io::ErrorKind::ConnectionAborted => {
                    println!(
                        "Lost connection reconnecting in {:?}...",
                        self.timeout
                    );
                    sleep(self.timeout);
                    let (recv, send) = configure_socket(self.addr)?;
                    *self.receiver.borrow_mut() = recv;
                    *self.sender.borrow_mut() = send;
                    (self.protocol)(&mut *self.inner.borrow_mut())?;
                    println!("Reconnected");
                }
                o => break o,
            }
        }
    }
}

impl<R> Read for Reconnect<R>
where
    R: Fn(&mut TcpStream) -> io::Result<()>,
{
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.do_reconnect(|s| s.read(buf))
    }

    fn read_vectored(
        &mut self,
        bufs: &mut [io::IoSliceMut],
    ) -> io::Result<usize> {
        self.do_reconnect(|s| s.read_vectored(bufs))
    }

    fn read_to_end(&mut self, buf: &mut Vec<u8>) -> io::Result<usize> {
        self.do_reconnect(|s| s.read_to_end(buf))
    }

    fn read_to_string(&mut self, buf: &mut String) -> io::Result<usize> {
        self.do_reconnect(|s| s.read_to_string(buf))
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.do_reconnect(|s| s.read_exact(buf))
    }
}

impl<R> Write for Reconnect<R>
where
    R: Fn(&mut TcpStream) -> io::Result<()>,
{
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.do_reconnect(|s| s.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.do_reconnect(|s| s.flush())
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice]) -> io::Result<usize> {
        self.do_reconnect(|s| s.write_vectored(bufs))
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.do_reconnect(|s| s.write_all(buf))
    }

    fn write_fmt(&mut self, fmt: fmt::Arguments<'_>) -> io::Result<()> {
        self.do_reconnect(|s| s.write_fmt(fmt))
    }
}
