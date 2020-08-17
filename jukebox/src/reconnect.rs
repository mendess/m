use std::{
    fmt,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    thread::sleep,
    time::Duration,
};

pub struct Reconnect {
    inner: TcpStream,
    addr: SocketAddr,
    timeout: Duration,
}

impl Reconnect {
    pub fn connect<A: ToSocketAddrs>(
        addr: A,
        timeout: Duration,
    ) -> io::Result<Self> {
        let addr = addr.to_socket_addrs()?.next().ok_or(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid socket address",
        ))?;
        Ok(Self {
            addr,
            inner: TcpStream::connect(addr)?,
            timeout,
        })
    }

    fn do_reconnect<F, T>(&mut self, mut f: F) -> io::Result<T>
    where
        F: FnMut(&mut TcpStream) -> io::Result<T>,
    {
        loop {
            match f(&mut self.inner) {
                Err(e) if e.kind() == io::ErrorKind::ConnectionAborted => {
                    println!(
                        "Lost connection reconnecting in {:?}...",
                        self.timeout
                    );
                    sleep(self.timeout);
                    self.inner = TcpStream::connect(self.addr)?;
                }
                o => break o,
            }
        }
    }

    pub fn split(self) -> io::Result<(Reconnect, Reconnect)> {
        let inner = self.inner.try_clone()?;
        Ok((Reconnect { inner, ..self }, self))
    }
}

impl Read for Reconnect {
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

impl Write for Reconnect {
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
