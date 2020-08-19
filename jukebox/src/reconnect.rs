use parking_lot::Mutex;
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    fmt,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    sync::Arc,
    thread::sleep,
    time::Duration,
};

pub const KEEP_ALIVE: Duration = Duration::from_secs(10);

pub struct Reconnect<R> {
    inner: Arc<Mutex<TcpStream>>,
    addr: SocketAddr,
    timeout: Duration,
    protocol: R,
}

fn configure_socket(addr: SocketAddr) -> io::Result<TcpStream> {
    let s = Socket::new(Domain::ipv4(), Type::stream(), Some(Protocol::tcp()))?;
    s.set_keepalive(Some(KEEP_ALIVE))?;
    s.connect(&addr.into())?;
    Ok(s.into_tcp_stream())
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
        Ok(Self {
            addr,
            inner: Arc::new(Mutex::new(configure_socket(addr)?)),
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
        let inner = Arc::clone(&self.inner);
        let protocol = self.protocol.clone();
        Ok((
            Reconnect {
                inner,
                protocol,
                ..self
            },
            self,
        ))
    }
}

impl<R> Reconnect<R>
where
    R: Fn(&mut TcpStream) -> io::Result<()>,
{
    fn do_reconnect<F, T: std::fmt::Debug>(&mut self, mut f: F) -> io::Result<T>
    where
        F: FnMut(&mut TcpStream) -> io::Result<T>,
    {
        loop {
            let e = { f(&mut *self.inner.lock()) };
            match dbg!(e) {
                Err(e) if e.kind() == io::ErrorKind::ConnectionAborted => {
                    println!(
                        "Lost connection reconnecting in {:?}...",
                        self.timeout
                    );
                    sleep(self.timeout);
                    *self.inner.lock() = dbg!(configure_socket(self.addr))?; // returns a :TcpStream
                    println!("running protocol");
                    (self.protocol)(&mut *self.inner.lock())?;
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
