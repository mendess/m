use crate::socket_channel::{self, Receiver as SReceiver, Sender as SSender};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{
    net::Ipv4Addr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    stream::StreamExt,
    sync::{
        mpsc::{self, Receiver, Sender},
        oneshot, Notify,
    },
};

#[derive(Debug)]
struct Room {
    requests: Sender<Message>,
    jukebox: Option<Jukebox>,
    counter: usize,
}

impl Room {
    fn new(name: String) -> Self {
        let (tx, rx) = mpsc::channel(64);
        Self {
            requests: tx,
            jukebox: Some(Jukebox::new(name, rx)),
            counter: 0,
        }
    }
}

#[derive(Debug, Default)]
pub struct Rooms {
    rooms: DashMap<String, Room>,
}

impl Rooms {
    pub fn list(&self) -> Vec<String> {
        self.rooms.iter().map(|kv| kv.key().clone()).collect()
    }

    pub async fn run_cmd<C>(&self, room: &str, cmd_line: C) -> bool
    where
        C: Into<Box<[String]>>,
    {
        match self.rooms.get_mut(room) {
            Some(mut r) => {
                let r = r.value_mut();
                if r.jukebox.is_none() {
                    if let Err(_) = r
                        .requests
                        .send(Message::Run(Request::new(cmd_line.into())))
                        .await
                    {
                        return false;
                    }
                    true
                } else {
                    false
                }
            }
            None => false,
        }
    }

    pub async fn get_cmd<C>(&self, room: &str, cmd_line: C) -> Option<String>
    where
        C: Into<Box<[String]>>,
    {
        match self.rooms.get_mut(room) {
            Some(mut r) if r.value().jukebox.is_none() => {
                let (tx, rx) = oneshot::channel();
                let r = r.value_mut();
                r.requests
                    .send(Message::Get(Request::new(cmd_line.into()), tx))
                    .await
                    .ok()?;
                rx.await.ok()
            }
            _ => None,
        }
    }

    fn create_jukebox(&self, name: String) -> Jukebox {
        let mut r = Room::new(name.clone());
        let jb = r.jukebox.take().unwrap();
        self.rooms.insert(name, r);
        jb
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    id: usize,
    cmd_line: Box<[String]>,
}

impl Request {
    fn new(cmd_line: Box<[String]>) -> Self {
        static REQUEST_COUNTER: AtomicUsize = AtomicUsize::new(0);
        Self {
            id: REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed),
            cmd_line,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    id: usize,
    response: String,
}

#[derive(Debug)]
enum Message {
    Run(Request),
    Get(Request, oneshot::Sender<String>),
    Reconnect(Arc<Notify>),
}

#[derive(Debug)]
struct Jukebox {
    name: String,
    channel: Receiver<Message>,
}

impl Jukebox {
    fn new(name: String, channel: Receiver<Message>) -> Self {
        Self { name, channel }
    }

    async fn handle<R, W>(
        &mut self,
        mut receiver: SReceiver<R>,
        mut sender: SSender<W>,
    ) -> io::Result<Option<Arc<Notify>>>
    where
        R: AsyncBufReadExt + Unpin,
        W: AsyncWriteExt + Unpin,
    {
        eprintln!("[J::handle::{}] Handling", self.name);
        let mut s = String::new();
        let gets = DashMap::new();
        loop {
            tokio::select! {
                Some(req) = self.channel.recv() => {
                    match req {
                        Message::Get(req, ch) => {
                            eprintln!(
                                "[J::handle::{}] sending request {:?} to remote",
                                self.name,
                                req
                            );
                            sender.asend(&req).await?;
                            eprintln!("[J::handle::{}] sent", self.name);
                            gets.insert(req.id, ch);
                        },
                        Message::Run(req) => {
                            eprintln!(
                                "[J::handle::{}] sending request {:?} to remote",
                                self.name,
                                req);
                            sender.asend(&req).await?;
                        },
                        Message::Reconnect(n) => {
                            eprintln!(
                                "[J::handle::{}] Terminating this intance as \
                                requested",
                                self.name
                            );
                            break Ok(Some(n))
                        }
                    }
                }
                r = receiver.arecv_with_buf::<Response>(&mut s) => {
                    let r = r?;
                    eprintln!("[J::handle::{}] got {:?} from remote", self.name, r);
                    if let Some((_, ch)) = gets.remove(&r.id) {
                        if let Err(_) = ch.send(r.response) {
                            eprintln!(
                                "[J::handle::{}] user went away, can't send \
                                result of command",
                                self.name
                            );
                        }
                    }
                    s.clear();
                }
                else => break Ok(None)
            }
        }
    }
}

async fn create_room<R, W>(
    rooms: &Rooms,
    receiver: &mut SReceiver<R>,
    sender: &mut SSender<W>,
) -> io::Result<Jukebox>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let mut s = String::new();
    let r = loop {
        let name = receiver.arecv_with_buf::<String>(&mut s).await?;
        {
            match rooms.rooms.get_mut(&name) {
                Some(mut room) => {
                    if let Some(jbox) = room.value_mut().jukebox.take() {
                        eprintln!("[J::{}] Reconnecting to existing room", s);
                        break Ok(jbox);
                    }
                }
                None => {
                    eprintln!("[J::{}] Creating new room", s);
                    break Ok(rooms.create_jukebox(s));
                }
            }
        }
        sender.asend(false).await?;
    };
    sender.asend(true).await?;
    r
}

async fn reconnect_room<R, W>(
    rooms: &Rooms,
    receiver: &mut SReceiver<R>,
    sender: &mut SSender<W>,
) -> io::Result<Jukebox>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let s = receiver.arecv::<String>().await?;
    let r = match rooms.rooms.get_mut(&s) {
        Some(mut room) => {
            if let Some(jbox) = room.value_mut().jukebox.take() {
                eprintln!(
                    "[J::{}] Reconnecting to existing room and jb already returned",
                    s
                );
                Ok(jbox)
            } else {
                let n = Arc::new(Notify::new());
                eprintln!("[J::{}] Terminating old task", s);
                match room
                    .requests
                    .send(Message::Reconnect(Arc::clone(&n)))
                    .await
                {
                    Ok(_) => {
                        n.notified().await;
                        rooms
                            .rooms
                            .get_mut(&s)
                            .and_then(|mut r| r.value_mut().jukebox.take())
                            .ok_or_else(|| {
                                io::Error::new(
                                io::ErrorKind::Other,
                                "jukebox disapeared when trying to reconnect"
                            )
                            })
                    }
                    Err(_) => {
                        eprintln!(
                            "[J::{}] Something terrible has happened. \
                                    The jukebox was dropped instead of \
                                    returned to the ROOMS variable",
                            s
                        );
                        eprintln!("[J::{}] Creating new room", s);
                        Ok(rooms.create_jukebox(s))
                    }
                }
            }
        }
        None => {
            eprintln!(
                "[J::{}] Trying to reconnect to a room that doesn't exist. \
                    Creating new room",
                s
            );
            Ok(rooms.create_jukebox(s))
        }
    };
    sender.asend(true).await?;
    r
}

#[derive(Serialize, Deserialize, Debug)]
pub enum Protocol {
    Jukebox,
    Reconnect,
}

async fn protocol<R, W>(
    rooms: &Rooms,
    receiver: &mut SReceiver<R>,
    sender: &mut SSender<W>,
) -> io::Result<Jukebox>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    match receiver.arecv().await? {
        Protocol::Jukebox => Ok(create_room(rooms, receiver, sender).await?),
        Protocol::Reconnect => {
            Ok(reconnect_room(rooms, receiver, sender).await?)
        }
    }
}

async fn handle(rooms: &Rooms, mut stream: TcpStream) -> io::Result<()> {
    eprintln!("New connection");
    let (reader, writer) = stream.split();
    let (mut receiver, mut sender) =
        socket_channel::make(BufReader::new(reader), writer);
    let mut jb = protocol(rooms, &mut receiver, &mut sender).await?;
    eprintln!("[J::{}] Handling", jb.name);
    let e = jb.handle(receiver, sender).await;
    match &e {
        Ok(_) => eprintln!("[J::{}] Jukebox left", jb.name),
        Err(e) => eprintln!("[J::{}] Jukebox left: {:?}", jb.name, e),
    }
    let name = jb.name.clone();
    eprintln!("[J::{}] returning jukebox to rooms", name);
    rooms
        .rooms
        .get_mut(&name)
        .map(|mut o| o.value_mut().jukebox = Some(jb));
    eprintln!("[J::{}] returned", name);
    e.map(|n| n.as_deref().map(Notify::notify)).map(|_| ())
}

pub static ROOMS: Lazy<Rooms> = Lazy::new(Default::default);

pub async fn start(port: u16) -> io::Result<()> {
    let mut listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await?;
    let mut incoming = listener.incoming();
    eprintln!("Socket server listening on port: {}", port);
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        stream.set_keepalive(Some(crate::reconnect::KEEP_ALIVE))?;
        tokio::spawn(handle(&*ROOMS, stream));
    }

    Ok(())
}
