use crate::net::socket_channel::{
    self, SocketChannelReceive, SocketChannelSend,
};
use serde::{de::DeserializeOwned, Serialize};
use socket2::{Domain, Protocol, Socket, Type};
use std::{
    cell::RefCell,
    io::{self, BufReader},
    net::{SocketAddr, TcpStream, ToSocketAddrs},
    rc::Rc,
    thread::sleep,
    time::Duration,
};

pub const KEEP_ALIVE: Duration = Duration::from_secs(10);

pub type Receiver = socket_channel::Receiver<BufReader<TcpStream>>;
pub type Sender = socket_channel::Sender<TcpStream>;

pub struct Reconnect<R> {
    receiver: Rc<RefCell<Receiver>>,
    sender: Rc<RefCell<Sender>>,
    addr: SocketAddr,
    timeout: Duration,
    protocol: R,
}

fn configure_socket(addr: SocketAddr) -> io::Result<(Receiver, Sender)> {
    let s = Socket::new(Domain::ipv4(), Type::stream(), Some(Protocol::tcp()))?;
    s.set_keepalive(Some(KEEP_ALIVE))?;
    s.connect(&addr.into())?;
    let tcp = s.into_tcp_stream();
    let (recv, send) =
        socket_channel::make(BufReader::new(tcp.try_clone()?), tcp);
    Ok((recv, send))
}

impl<R> Reconnect<R>
where
    R: Fn(&mut Receiver, &mut Sender) -> io::Result<()>,
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
            receiver: Rc::new(RefCell::new(recv)),
            sender: Rc::new(RefCell::new(send)),
            timeout,
            protocol,
        })
    }
}

impl<R> Reconnect<R>
where
    R: Fn(&mut Receiver, &mut Sender) -> io::Result<()>,
    R: Clone,
{
    pub fn split(self) -> io::Result<(Self, Self)> {
        let receiver = Rc::clone(&self.receiver);
        let sender = Rc::clone(&self.sender);
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
    R: Fn(&mut Receiver, &mut Sender) -> io::Result<()>,
{
    fn do_reconnect<F, T>(&mut self, mut f: F) -> io::Result<T>
    where
        F: FnMut(&mut Receiver, &mut Sender) -> io::Result<T>,
    {
        loop {
            match {
                f(
                    &mut *self.receiver.borrow_mut(),
                    &mut *self.sender.borrow_mut(),
                )
            } {
                Err(e) if e.kind() == io::ErrorKind::ConnectionAborted => {
                    println!(
                        "Lost connection reconnecting in {:?}...",
                        self.timeout
                    );
                    sleep(self.timeout);
                    let (recv, send) = configure_socket(self.addr)?;
                    *self.receiver.borrow_mut() = recv;
                    *self.sender.borrow_mut() = send;
                    (self.protocol)(
                        &mut *self.receiver.borrow_mut(),
                        &mut *self.sender.borrow_mut(),
                    )?;
                    println!("Reconnected");
                }
                o => break o,
            }
        }
    }
}

impl<R> SocketChannelReceive for Reconnect<R>
where
    R: Fn(&mut Receiver, &mut Sender) -> io::Result<()>,
{
    fn recv_with_buf<M: DeserializeOwned>(
        &mut self,
        buf: &mut String,
    ) -> io::Result<M> {
        self.do_reconnect(|recv, _| recv.recv_with_buf(buf))
    }
}

impl<R> SocketChannelSend for Reconnect<R>
where
    R: Fn(&mut Receiver, &mut Sender) -> io::Result<()>,
{
    fn send<M: Serialize>(&mut self, m: M) -> io::Result<()> {
        self.do_reconnect(|_, sender| sender.send(&m))
    }
}
