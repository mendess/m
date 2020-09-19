use crate::{
    net::{
        reconnect::KEEP_ALIVE,
        socket_channel::{self, Receiver as SReceiver, Sender as SSender},
    },
    relay::rooms::{Jukebox, Message, Response, Rooms},
    RoomName,
};
use dashmap::DashMap;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::{net::Ipv4Addr, sync::Arc};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    stream::StreamExt,
    sync::Notify,
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
impl Jukebox {
    async fn handle<R, W>(
        &mut self,
        mut receiver: SReceiver<R>,
        mut sender: SSender<W>,
    ) -> io::Result<Option<Arc<Notify>>>
    where
        R: AsyncBufReadExt + Unpin,
        W: AsyncWriteExt + Unpin,
    {
        log!(@self.name(), "Handling");
        let mut s = String::new();
        let gets = DashMap::new();
        loop {
            tokio::select! {
                Some(req) = self.recv() => {
                    match req {
                        Message::Get(req, ch) => {
                            log!(
                                @self.name(),
                                "sending get request {:?} to remote",
                                req
                            );
                            sender.asend(&req).await?;
                            gets.insert(req.id, ch);
                        },
                        Message::Run(req) => {
                            log!(@self.name(), "sending run request {:?} to remote", req);
                            sender.asend(&req).await?;
                        },
                        Message::Reconnect(n) => {
                            log!(@self.name(), "Terminating this intance as requested");
                            break Ok(Some(n))
                        }
                    }
                }
                r = receiver.arecv_with_buf::<Response>(&mut s) => {
                    let r = r?;
                    log!(@self.name(), "got {:?} from remote", r);
                    if let Some((_, ch)) = gets.remove(&r.id) {
                        if let Err(_) = ch.send(r.response) {
                            log!(
                                @self.name(),
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
            match rooms.get_mut(&name) {
                Some(mut room) => {
                    if let Some(jbox) = room.take_box() {
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
    let r = match rooms.get_mut(&s) {
        Some(mut room) => {
            if let Some(jbox) = room.take_box() {
                log!(@s, "Reconnecting to existing room and jb already returned");
                Ok(jbox)
            } else {
                let n = Arc::new(Notify::new());
                log!(@s, "Terminating old task");
                match room.request(Message::Reconnect(Arc::clone(&n))).await {
                    Ok(_) => {
                        n.notified().await;
                        rooms
                            .get_mut(&s)
                            .and_then(|mut r| r.take_box())
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
        Ok(_) => log!(@jb.name(), "Jukebox left"),
        Err(e) => log!(@jb.name(), "Jukebox left: {:?}", e),
    }
    let name = jb.name().clone();
    log!(@name, "returning jukebox to rooms");
    rooms.get_mut(&name).map(|mut o| o.set_box(jb));
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
