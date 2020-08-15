pub mod admin;
pub mod jukebox;
pub mod user;

use itertools::Itertools;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, marker::Unpin, net::Ipv4Addr, sync::Mutex};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter},
    net::{TcpListener, TcpStream},
    prelude::*,
    stream::StreamExt,
    sync::mpsc::{self, Receiver, Sender},
};

#[derive(Debug)]
struct State {
    channel: Sender<Message>,
    counter: usize,
}

impl From<Sender<Message>> for State {
    fn from(channel: Sender<Message>) -> Self {
        Self {
            channel,
            counter: Default::default(),
        }
    }
}

type Rooms = HashMap<String, State>;
static ROOMS: Lazy<Mutex<Rooms>> = Lazy::new(Mutex::default);

#[derive(Debug)]
enum Message {
    Register(usize, Sender<Response>),
    Request(Request),
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
            let mut rooms = ROOMS.lock().unwrap();
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

    async fn handle<R, W>(mut self, mut reader: R, mut writer: W) -> io::Result<()>
    where
        R: AsyncBufReadExt + Unpin,
        W: AsyncWrite + Unpin,
    {
        println!("Handling user");
        let mut s = String::new();
        while {
            s.clear();
            reader.read_line(&mut s).await? > 0
        } {
            println!("[U::{}] requesting {:?}", self.id, s);
            s.pop();
            if let Err(_) = self
                .requests
                .send(Message::Request(Request {
                    id: self.id,
                    s: s.clone(),
                }))
                .await
            {
                //TODO: Inform user that jukebox has left
                println!("[U::{}] jukebox left", self.id);
                break;
            }
            let r = match self.responses.recv().await {
                Some(r) => r,
                None => break,
            };
            println!("[U::{}] responding with '{:?}'", self.id, r);
            writer
                .write_all(serde_json::to_string(&r.data)?.as_bytes())
                .await?;
        }
        Ok(())
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

    async fn handle<R, W>(mut self, mut reader: R, mut writer: W) -> io::Result<()>
    where
        R: AsyncBufReadExt + Unpin,
        W: AsyncWrite + Unpin,
    {
        println!("[J::{}] Handling", self.name);
        let mut s = String::new();
        loop {
            tokio::select! {
                Some(req) = self.channel.recv() => {
                    match req {
                        Message::Request(req) => {
                            println!("[J::{}] sending request {:?} to remote", self.name, req);
                            writer
                                .write_all(serde_json::to_string(&req)?.as_bytes())
                                .await?;
                        }
                        Message::Register(id, ch) => {
                            println!("[J::{}] Registering {}", self.name, id);
                            self.users.insert(id, ch);
                        }
                    }
                }
                o = reader.read_line(&mut s) => {
                    o?;
                    s.pop();
                    let r = serde_json::from_str::<Response>(&s)?;
                    println!("[J::{}] got {:?} from remote", self.name, r);
                    let sender = match self.users.get_mut(&r.id) {
                        Some(s) => s,
                        None => continue,
                    };
                    if let Err(_) = sender.send(r).await {
                        println!("[J::{}] user went away, can't send result of command", self.name);
                    }
                    s.clear();
                }
                else => break Ok(())
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
        async fn send<W>(mut writer: W, response: Result<String, String>) -> io::Result<()>
        where
            W: AsyncWrite + Unpin,
        {
            let r = serde_json::to_string(&response)?;
            writer.write_all(r.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
            Ok(())
        };
        while reader.read_line(&mut s).await? > 0 {
            match s.trim() {
                "rooms" => {
                    send(
                        &mut writer,
                        Ok({
                            let s: String = ROOMS.lock().unwrap().keys().join("\n");
                            s
                        }),
                    )
                    .await?
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
    pub fn command(&self) -> impl Iterator<Item = &str> {
        self.s
            .split_whitespace()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
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
        if ROOMS.lock().unwrap().contains_key(&s) {
            break;
        }
        writer.write_all(&[false as u8]).await?;
    }
    Ok(s)
}

async fn create_room<R, W>(mut reader: R, mut writer: W) -> io::Result<(String, Receiver<Message>)>
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
            let mut guard = ROOMS.lock().unwrap();
            if !guard.contains_key(&s) {
                let (tx, rx) = mpsc::channel(64);
                guard.insert(s.clone(), tx.into());
                break Ok((s, rx));
            }
        }
        writer.write_all(&[false as u8]).await?;
    }
}

async fn protocol<R, W>(mut reader: R, mut writer: W) -> io::Result<Kind>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut s = String::with_capacity(7);
    reader.read_line(&mut s).await?;
    let kind = match s.trim() {
        "user" => {
            let name = get_name(&mut reader, &mut writer).await?;
            match User::new(&name).await {
                Some(u) => Ok(Kind::User(u)),
                None => return Err(io::Error::new(io::ErrorKind::Other, "jukebox is gone")),
            }
        }
        "jukebox" => {
            let (name, ch) = create_room(&mut reader, &mut writer).await?;
            Ok(Kind::Jukebox(Jukebox::new(name, ch)))
        }
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
        Kind::User(user) => user.handle(reader, writer).await?,
        Kind::Jukebox(jb) => jb.handle(reader, writer).await?,
        Kind::Admin => Admin.handle(reader, writer).await?,
    }
    Ok(())
}

pub async fn run(port: u16) -> io::Result<()> {
    let mut listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await?;
    let mut incoming = listener.incoming();
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        tokio::spawn(async {
            if let Err(e) = handle(stream).await {
                eprintln!("Handing failed with {}", e);
            }
        });
    }
    Ok(())
}
