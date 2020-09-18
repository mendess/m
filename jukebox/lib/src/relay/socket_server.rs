use crate::{
    net::{
        reconnect::KEEP_ALIVE,
        socket_channel::{self, Receiver as SReceiver, Sender as SSender},
    },
    RoomName,
};
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

macro_rules! function_name {
    () => {{
        fn f() {}
        fn type_name_of<T>(_: T) -> &'static str {
            std::any::type_name::<T>()
        }
        let name = type_name_of(f);
        &name[..name.len() - 3]
            .rsplit("::")
            .skip_while(|s| s.contains("closure"))
            .next()
            .unwrap_or("")
    }};
}
macro_rules! log {
    ($msg:expr $(,$args:expr)*$(,)?) => {
        ::std::eprintln!(
            "[{}::{}] {}",
            ::chrono::offset::Utc::now().format("%m/%d-%H:%M"),
            function_name!(),
            format!($msg, $($args,)*)
        )
    };
    (@$name:expr, $msg:expr $(,$args:expr)*$(,)?) => {
        ::std::eprintln!(
            "[{}::{}::{}] {}",
            ::chrono::offset::Utc::now().format("%m/%d-%H:%M"),
            $name,
            function_name!(),
            format!($msg, $($args,)*)
        )
    };
}

#[derive(Debug)]
struct Room {
    requests: Sender<Message>,
    jukebox: Option<Jukebox>,
    counter: usize,
}

impl Room {
    fn new(name: RoomName) -> Self {
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
    rooms: DashMap<RoomName, Room>,
}

impl Rooms {
    pub fn list(&self) -> Vec<String> {
        self.rooms.iter().map(|kv| kv.key().name.clone()).collect()
    }

    pub async fn run_cmd<C>(&self, room: &RoomName, cmd_line: C) -> bool
    where
        C: Into<Box<[String]>>,
    {
        match self.rooms.get_mut(room) {
            Some(mut r) => {
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

    pub async fn get_cmd<C>(
        &self,
        room: &RoomName,
        cmd_line: C,
    ) -> Option<Result<String, String>>
    where
        C: Into<Box<[String]>>,
    {
        match self.rooms.get_mut(room) {
            Some(mut r) if r.jukebox.is_none() => {
                let (tx, rx) = oneshot::channel();
                r.requests
                    .send(Message::Get(Request::new(cmd_line.into()), tx))
                    .await
                    .ok()?;
                rx.await.ok()
            }
            _ => None,
        }
    }

    fn create_jukebox(&self, name: RoomName) -> Jukebox {
        let mut r = Room::new(name.clone());
        let jb = r.jukebox.take().unwrap();
        self.rooms.insert(name, r);
        jb
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    id: usize,
    pub cmd_line: Box<[String]>,
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
    response: Result<String, String>,
}

impl Response {
    pub fn new(r: Request, response: Result<String, String>) -> Self {
        Self { id: r.id, response }
    }
}

#[derive(Debug)]
enum Message {
    Run(Request),
    Get(Request, oneshot::Sender<Result<String, String>>),
    Reconnect(Arc<Notify>),
}

#[derive(Debug)]
struct Jukebox {
    name: RoomName,
    channel: Receiver<Message>,
}

impl Jukebox {
    fn new(name: RoomName, channel: Receiver<Message>) -> Self {
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
        log!(@self.name, "Handling");
        let mut s = String::new();
        let gets = DashMap::new();
        loop {
            tokio::select! {
                Some(req) = self.channel.recv() => {
                    match req {
                        Message::Get(req, ch) => {
                            log!(
                                @self.name,
                                "sending get request {:?} to remote",
                                req
                            );
                            sender.asend(&req).await?;
                            gets.insert(req.id, ch);
                        },
                        Message::Run(req) => {
                            log!(@self.name, "sending run request {:?} to remote", req);
                            sender.asend(&req).await?;
                        },
                        Message::Reconnect(n) => {
                            log!(@self.name, "Terminating this intance as requested");
                            break Ok(Some(n))
                        }
                    }
                }
                r = receiver.arecv_with_buf::<Response>(&mut s) => {
                    let r = r?;
                    log!(@self.name, "got {:?} from remote", r);
                    if let Some((_, ch)) = gets.remove(&r.id) {
                        if let Err(_) = ch.send(r.response) {
                            log!(
                                @self.name,
                                "user went away, can't send result of command",
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
        let name = receiver.arecv_with_buf::<RoomName>(&mut s).await?;
        {
            match rooms.rooms.get_mut(&name) {
                Some(mut room) => {
                    if let Some(jbox) = room.jukebox.take() {
                        log!(@name, "Reconnecting to existing room");
                        break Ok(jbox);
                    }
                }
                None => {
                    log!(@name, "Creating new room");
                    break Ok(rooms.create_jukebox(name));
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
    let s = receiver.arecv::<RoomName>().await?;
    let r = match rooms.rooms.get_mut(&s) {
        Some(mut room) => {
            if let Some(jbox) = room.jukebox.take() {
                log!(@s, "Reconnecting to existing room and jb already returned");
                Ok(jbox)
            } else {
                let n = Arc::new(Notify::new());
                log!(@s, "Terminating old task");
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
                            .and_then(|mut r| r.jukebox.take())
                            .ok_or_else(|| {
                                io::Error::new(
                                io::ErrorKind::Other,
                                "jukebox disapeared when trying to reconnect"
                            )
                            })
                    }
                    Err(_) => {
                        log!(
                            @s,
                            "Something terrible has happened. \
                                    The jukebox was dropped instead of \
                                    returned to the ROOMS variable",
                        );
                        log!(@s, "Creating new room");
                        Ok(rooms.create_jukebox(s.to_owned()))
                    }
                }
            }
        }
        None => {
            log!(
                @s,
                "Trying to reconnect to a room that doesn't exist. \
                    Creating new room",
            );
            Ok(rooms.create_jukebox(s.to_owned()))
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

async fn handle_conn(rooms: &Rooms, mut stream: TcpStream) -> io::Result<()> {
    log!("New connection");
    let (reader, writer) = stream.split();
    let (mut receiver, mut sender) =
        socket_channel::make(BufReader::new(reader), writer);
    let mut jb = match receiver.arecv().await? {
        Protocol::Jukebox => {
            create_room(rooms, &mut receiver, &mut sender).await?
        }
        Protocol::Reconnect => {
            reconnect_room(rooms, &mut receiver, &mut sender).await?
        }
    };
    let e = jb.handle(receiver, sender).await;
    match &e {
        Ok(_) => log!(@jb.name, "Jukebox left"),
        Err(e) => log!(@jb.name, "Jukebox left: {:?}", e),
    }
    let name = jb.name.clone();
    log!(@name, "returning jukebox to rooms");
    rooms.rooms.get_mut(&name).map(|mut o| o.jukebox = Some(jb));
    log!(@name, "returned");
    e.map(|n| n.as_deref().map(Notify::notify)).map(|_| ())
}

pub static ROOMS: Lazy<Rooms> = Lazy::new(Default::default);

pub async fn start(port: u16) -> io::Result<()> {
    let mut listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, port)).await?;
    let mut incoming = listener.incoming();
    log!("Socket server listening on port: {}", port);
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        stream.set_keepalive(Some(KEEP_ALIVE))?;
        tokio::spawn(handle_conn(&*ROOMS, stream));
    }

    Ok(())
}
