use super::{Protocol, Request, Response};
use crate::socket_channel::{self, Receiver as SReceiver, Sender as SSender};
use itertools::Itertools;
use once_cell::sync::Lazy;
use std::{collections::HashMap, net::Ipv4Addr, sync::Arc};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    stream::StreamExt,
    sync::{
        mpsc::{self, Receiver, Sender},
        Mutex, Notify,
    },
};

#[derive(Debug)]
struct Room {
    channel: Sender<Message>,
    jukebox: Option<Jukebox>,
    counter: usize,
}

impl From<Sender<Message>> for Room {
    fn from(channel: Sender<Message>) -> Self {
        Self {
            channel,
            jukebox: Default::default(),
            counter: Default::default(),
        }
    }
}

type Rooms = HashMap<String, Room>;
static ROOMS: Lazy<Mutex<Rooms>> = Lazy::new(Mutex::default);

#[derive(Debug)]
enum Message {
    Register(usize, Sender<Response>),
    Request(Request),
    Reconnect(Arc<Notify>),
    Leave(usize),
}

#[derive(Debug)]
struct User {
    id: usize,
    requests: Sender<Message>,
    responses: Receiver<Response>,
}

impl User {
    fn new(
        id: usize,
        requests: Sender<Message>,
        responses: Receiver<Response>,
    ) -> Self {
        Self {
            id,
            requests,
            responses,
        }
    }

    async fn handle<R, W>(
        &mut self,
        mut receiver: SReceiver<R>,
        mut sender: SSender<W>,
    ) -> io::Result<()>
    where
        R: AsyncBufReadExt + Unpin,
        W: AsyncWriteExt + Unpin,
    {
        eprintln!("[U::handle::{}] Handling", self.id);
        loop {
            // TODO: select and use an id per msg
            let msg = receiver.arecv::<String>().await?;
            eprintln!("[U::handle::{}] requesting {:?}", self.id, msg);
            if let Err(_) = self
                .requests
                .send(Message::Request(Request {
                    id: self.id,
                    s: msg,
                }))
                .await
            {
                eprintln!("[U::handle::{}] jukebox left", self.id);
                sender
                    .asend(Result::<String, String>::Err(String::from(
                        "Jukebox left",
                    )))
                    .await?;
                break;
            }
            let r = match self.responses.recv().await {
                Some(r) => r,
                None => break,
            };
            eprintln!("[U::handle::{}] responding with '{:?}'", self.id, r);
            sender.asend(&r.data).await?;
        }
        eprintln!("[U::handle::{}] user left", self.id);
        Ok(())
    }
}

impl Drop for User {
    fn drop(&mut self) {
        let mut requests = self.requests.clone();
        let id = self.id;
        tokio::spawn(async move { requests.send(Message::Leave(id)).await });
    }
}

#[derive(Debug)]
struct Jukebox {
    name: String,
    channel: Receiver<Message>,
    users: HashMap<usize, Sender<Response>>,
}

impl Jukebox {
    fn new(name: String, channel: Receiver<Message>) -> Self {
        Self {
            name,
            channel,
            users: Default::default(),
        }
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
        loop {
            tokio::select! {
                Some(req) = self.channel.recv() => {
                    match req {
                        Message::Request(req) => {
                            eprintln!(
                                "[J::handle::{}] sending request {:?} to remote",
                                self.name,
                                req
                            );
                            sender.asend(&req).await?;
                            eprintln!("[J::handle::{}] sent", self.name)
                        }
                        Message::Register(id, ch) => {
                            eprintln!("[J::handle::{}] Registering {}", self.name, id);
                            self.users.insert(id, ch);
                        }
                        Message::Leave(id) => {
                            eprintln!("[J::handle::{}] Removing user {}", self.name, id);
                            self.users.remove(&id);
                        }
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
                    let sender = match self.users.get_mut(&r.id) {
                        Some(s) => s,
                        None => continue,
                    };
                    if let Err(_) = sender.send(r).await {
                        eprintln!(
                            "[J::handle::{}] user went away, can't send \
                            result of command",
                            self.name
                        );
                    }
                    s.clear();
                }
                else => break Ok(None)
            }
        }
    }
}

struct Admin;

impl Admin {
    async fn handle<R, W>(
        self,
        mut receiver: SReceiver<R>,
        mut sender: SSender<W>,
    ) -> io::Result<()>
    where
        R: AsyncBufReadExt + Unpin,
        W: AsyncWriteExt + Unpin,
    {
        eprintln!("[A] Handling");
        let mut s = String::new();
        loop {
            let msg = receiver.arecv_with_buf::<String>(&mut s).await?;
            match msg.trim() {
                "rooms" => {
                    let s = ROOMS.lock().await.keys().join("\n");
                    sender.asend::<Result<_, ()>>(Ok(s)).await?
                }
                _ => {
                    sender
                        .asend::<Result<(), _>>(Err("Invalid command"))
                        .await?
                }
            }
        }
    }
}

#[derive(Debug)]
enum Kind {
    Jukebox(Jukebox),
    User(User),
    Admin,
}

async fn get_name<R, W>(
    receiver: &mut SReceiver<R>,
    sender: &mut SSender<W>,
) -> io::Result<String>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let mut s = String::new();
    loop {
        let msg = receiver.arecv_with_buf::<String>(&mut s).await?;
        if ROOMS.lock().await.contains_key(&msg) {
            break;
        }
        sender.asend(false).await?;
    }
    sender.asend(true).await?;
    Ok(s)
}

async fn create_room<R, W>(
    receiver: &mut SReceiver<R>,
    sender: &mut SSender<W>,
) -> io::Result<Jukebox>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let mut s = String::new();
    let r = loop {
        let msg = receiver.arecv_with_buf::<String>(&mut s).await?;
        {
            let mut guard = ROOMS.lock().await;
            match guard.get_mut(&msg) {
                Some(room) => {
                    if let Some(jbox) = room.jukebox.take() {
                        eprintln!("[J::{}] Reconnecting to existing room", s);
                        break Ok(jbox);
                    }
                }
                None => {
                    let (tx, rx) = mpsc::channel(64);
                    guard.insert(s.clone(), tx.into());
                    eprintln!("[J::{}] Creating new room", s);
                    break Ok(Jukebox::new(s, rx));
                }
            }
        }
        sender.asend(false).await?;
    };
    sender.asend(true).await?;
    r
}

async fn reconnect_room<R, W>(
    receiver: &mut SReceiver<R>,
    sender: &mut SSender<W>,
) -> io::Result<Jukebox>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    let s = receiver.arecv::<String>().await?;
    let mut guard = ROOMS.lock().await;
    let r = match guard.get_mut(&s) {
        Some(room) => {
            if let Some(jbox) = room.jukebox.take() {
                eprintln!(
                    "[J::{}] Reconnecting to existing room and jb already returned",
                    s
                );
                Ok(jbox)
            } else {
                let n = Arc::new(Notify::new());
                eprintln!("[J::{}] Terminating old task", s);
                match room
                    .channel
                    .send(Message::Reconnect(Arc::clone(&n)))
                    .await
                {
                    Ok(_) => {
                        drop(guard);
                        n.notified().await;
                        ROOMS
                            .lock()
                            .await
                            .get_mut(&s)
                            .and_then(|r| r.jukebox.take())
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
                        let (tx, rx) = mpsc::channel(64);
                        guard.insert(s.clone(), tx.into());
                        eprintln!("[J::{}] Creating new room", s);
                        Ok(Jukebox::new(s, rx))
                    }
                }
            }
        }
        None => {
            let (tx, rx) = mpsc::channel(64);
            guard.insert(s.clone(), tx.into());
            eprintln!(
                "[J::{}] Trying to reconnect to a room that doesn't exist. \
                    Creating new room",
                s
            );
            Ok(Jukebox::new(s, rx))
        }
    };
    sender.asend(true).await?;
    r
}

async fn protocol<R, W>(
    receiver: &mut SReceiver<R>,
    sender: &mut SSender<W>,
) -> io::Result<Kind>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWriteExt + Unpin,
{
    match receiver.arecv().await? {
        Protocol::User => {
            let name = get_name(receiver, sender).await?;
            let (id, mut requests) = {
                let mut rooms = ROOMS.lock().await;
                let mut state = match rooms.get_mut(&name) {
                    Some(state) => state,
                    None => {
                        return Err(io::Error::new(
                            io::ErrorKind::Other,
                            "jukebox is gone",
                        ))
                    }
                };
                let id = state.counter;
                state.counter += 1;
                (id, state.channel.clone())
            };
            let (tx, rx) = mpsc::channel(2);
            if let Err(_) = requests.send(Message::Register(id, tx)).await {
                return Err(io::Error::new(
                    io::ErrorKind::Other,
                    "jukebox is gone",
                ));
            }
            Ok(Kind::User(User::new(id, requests, rx)))
        }
        Protocol::Jukebox => {
            Ok(Kind::Jukebox(create_room(receiver, sender).await?))
        }
        Protocol::Reconnect => {
            Ok(Kind::Jukebox(reconnect_room(receiver, sender).await?))
        }
        Protocol::Admin => Ok(Kind::Admin),
    }
}

async fn handle(mut stream: TcpStream) -> io::Result<()> {
    let (reader, writer) = stream.split();
    let (mut receiver, mut sender) =
        socket_channel::make(BufReader::new(reader), writer);
    let k = protocol(&mut receiver, &mut sender).await?;
    match k {
        Kind::User(mut user) => {
            eprintln!("[U::{}] Handling", user.id);
            let e = user.handle(receiver, sender).await;
            match &e {
                Ok(_) => eprintln!("[U::{}] User left", user.id),
                Err(e) => eprintln!("[U::{}] User left: {:?}", user.id, e),
            }
            e
        }
        Kind::Jukebox(mut jb) => {
            eprintln!("[J::{}] Handling", jb.name);
            let e = jb.handle(receiver, sender).await;
            match &e {
                Ok(_) => eprintln!("[J::{}] Jukebox left", jb.name),
                Err(e) => eprintln!("[J::{}] Jukebox left: {:?}", jb.name, e),
            }
            let name = jb.name.clone();
            eprintln!("[J::{}] returning jukebox to rooms", name);
            ROOMS
                .lock()
                .await
                .get_mut(&name)
                .map(|o| o.jukebox = Some(jb));
            eprintln!("[J::{}] returned", name);
            e.map(|n| n.as_deref().map(Notify::notify)).map(|_| ())
        }
        Kind::Admin => Admin.handle(receiver, sender).await,
    }
}

pub async fn run(port: u16) -> io::Result<()> {
    let mut listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await?;
    let mut incoming = listener.incoming();
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        stream.set_keepalive(Some(crate::reconnect::KEEP_ALIVE))?;
        tokio::spawn(handle(stream));
    }
    Ok(())
}
