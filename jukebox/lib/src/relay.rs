pub mod admin;
pub mod jukebox;
pub mod user;
pub mod client_util;

use itertools::Itertools;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, marker::Unpin, net::Ipv4Addr, sync::Arc};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{TcpListener, TcpStream},
    prelude::*,
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
    async fn new(name: &str) -> Option<Self> {
        let (id, mut requests) = {
            let mut rooms = ROOMS.lock().await;
            let mut state = rooms.get_mut(name)?;
            let id = state.counter;
            state.counter += 1;
            (id, state.channel.clone())
        };
        let (tx, rx) = mpsc::channel(2);
        requests.send(Message::Register(id, tx)).await.ok()?;
        Some(Self {
            id,
            requests,
            responses: rx,
        })
    }

    async fn handle<R, W>(
        &mut self,
        mut reader: R,
        mut writer: W,
    ) -> io::Result<()>
    where
        R: AsyncBufReadExt + Unpin,
        W: AsyncWrite + Unpin,
    {
        eprintln!("[U::handle::{}] Handling", self.id);
        let mut s = String::new();
        while {
            s.clear();
            reader.read_line(&mut s).await? > 0
        } {
            eprintln!("[U::handle::{}] requesting {:?}", self.id, s);
            s.pop();
            if s.is_empty() {
                continue;
            }
            if let Err(_) = self
                .requests
                .send(Message::Request(Request {
                    id: self.id,
                    s: s.clone(),
                }))
                .await
            {
                eprintln!("[U::handle::{}] jukebox left", self.id);
                writer
                    .write_all(
                        serde_json::to_string(&Result::<String, String>::Err(
                            String::from("Jukebox left"),
                        ))?
                        .as_bytes(),
                    )
                    .await?;
                break;
            }
            let r = match self.responses.recv().await {
                Some(r) => r,
                None => break,
            };
            eprintln!("[U::handle::{}] responding with '{:?}'", self.id, r);
            writer
                .write_all(serde_json::to_string(&r.data)?.as_bytes())
                .await?;
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
        mut reader: R,
        mut writer: W,
    ) -> io::Result<Option<Arc<Notify>>>
    where
        R: AsyncBufReadExt + Unpin,
        W: AsyncWrite + Unpin,
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
                            writer
                                .write_all(serde_json::to_string(&req)?.as_bytes())
                                .await?;
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
                o = reader.read_line(&mut s) => {
                    match o? {
                        0 => break Ok(None),
                        1 => continue,
                        _ => (),
                    }
                    s.pop();
                    let r = serde_json::from_str::<Response>(&s)?;
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
    async fn handle<W, R>(self, mut reader: R, writer: W) -> io::Result<()>
    where
        R: AsyncBufReadExt + Unpin,
        W: AsyncWrite + Unpin,
    {
        eprintln!("[A] Handling");
        let mut writer = BufWriter::new(writer);
        let mut s = String::new();
        async fn send<W>(
            mut writer: W,
            response: Result<String, String>,
        ) -> io::Result<()>
        where
            W: AsyncWrite + Unpin,
        {
            let r = dbg!(serde_json::to_string(&response)?);
            writer.write_all(r.as_bytes()).await?;
            writer.flush().await?;
            Ok(())
        };
        while reader.read_line(&mut s).await? > 0 {
            match s.trim() {
                "rooms" => {
                    let s = ROOMS.lock().await.keys().join("\n");
                    send(&mut writer, Ok(s)).await?
                }
                _ => send(&mut writer, Err("Invalid command".into())).await?,
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
enum Kind {
    Jukebox(Jukebox),
    User(User),
    Admin,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    id: usize,
    s: String,
}

impl Request {
    pub fn command(&self) -> Vec<&str> {
        crate::arg_split::quoted_parse(&self.s)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    id: usize,
    data: Result<String, String>,
}

impl Response {
    pub fn new(r: Request, data: Result<String, String>) -> Self {
        Self { id: r.id, data }
    }
}

async fn get_name<R, W>(mut reader: R, mut writer: W) -> io::Result<String>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut s = String::new();
    loop {
        s.clear();
        reader.read_line(&mut s).await?;
        s.pop();
        if ROOMS.lock().await.contains_key(&s) {
            break;
        }
        writer.write_all(&[false as u8]).await?;
    }
    Ok(s)
}

async fn create_room<R, W>(mut reader: R, mut writer: W) -> io::Result<Jukebox>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut s = String::new();
    loop {
        s.clear();
        reader.read_line(&mut s).await?;
        s.pop();
        {
            let mut guard = ROOMS.lock().await;
            match guard.get_mut(&s) {
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
        writer.write_all(&[false as u8]).await?;
    }
}

async fn reconnect_room<R>(mut reader: R) -> io::Result<Jukebox>
where
    R: AsyncBufReadExt + Unpin,
{
    let mut s = String::new();
    reader.read_line(&mut s).await?;
    s.pop();
    let mut guard = ROOMS.lock().await;
    match guard.get_mut(&s) {
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
    }
}

async fn protocol<R, W>(mut reader: R, mut writer: W) -> io::Result<Kind>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut s = String::with_capacity(7);
    reader.read_line(&mut s).await?;
    let kind = match dbg!(s.trim()) {
        "user" => {
            let name = get_name(&mut reader, &mut writer).await?;
            match User::new(&name).await {
                Some(u) => Ok(Kind::User(u)),
                None => {
                    return Err(io::Error::new(
                        io::ErrorKind::Other,
                        "jukebox is gone",
                    ))
                }
            }
        }
        "jukebox" => {
            Ok(Kind::Jukebox(create_room(&mut reader, &mut writer).await?))
        }
        "reconnect" => Ok(Kind::Jukebox(reconnect_room(&mut reader).await?)),
        "admin" => Ok(Kind::Admin),
        _ => Err(io::Error::new(io::ErrorKind::Other, "Invalid user kind")),
    }?;
    writer.write_all(&[true as u8]).await?;
    Ok(kind)
}

async fn handle(mut stream: TcpStream) -> io::Result<()> {
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);
    let k = protocol(&mut reader, &mut writer).await?;
    match k {
        Kind::User(mut user) => {
            eprintln!("[U::{}] Handling", user.id);
            let e = user.handle(reader, writer).await;
            match &e {
                Ok(_) => eprintln!("[U::{}] User left", user.id),
                Err(e) => eprintln!("[U::{}] User left: {:?}", user.id, e),
            }
            e
        }
        Kind::Jukebox(mut jb) => {
            eprintln!("[J::{}] Handling", jb.name);
            let e = jb.handle(reader, writer).await;
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
        Kind::Admin => Admin.handle(reader, writer).await,
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