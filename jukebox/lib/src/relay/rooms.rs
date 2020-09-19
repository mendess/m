use crate::RoomName;
use dashmap::{
    mapref::one::{Ref, RefMut},
    DashMap,
};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::sync::{
    mpsc::{self, error::SendError, Receiver, Sender},
    oneshot, Notify,
};

#[derive(Debug)]
pub struct Room {
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

    pub fn take_box(&mut self) -> Option<Jukebox> {
        self.jukebox.take()
    }

    pub fn set_box(&mut self, b: Jukebox) {
        self.jukebox = Some(b)
    }

    pub async fn request(
        &mut self,
        m: Message,
    ) -> Result<(), SendError<Message>> {
        self.requests.send(m).await
    }
}

#[derive(Debug, Default)]
pub struct Rooms {
    rooms: DashMap<RoomName, Room>,
}

impl Rooms {
    pub fn list(&self) -> Vec<(String, bool)> {
        self.rooms
            .iter()
            .map(|kv| (kv.key().name.clone(), kv.value().jukebox.is_none()))
            .collect()
    }

    pub async fn run_cmd<C>(&self, room: &RoomName, cmd_line: C) -> bool
    where
        C: Into<Box<[String]>>,
    {
        match self.get_mut(room) {
            Some(mut r) => {
                if r.jukebox.is_none() {
                    if let Err(_) = r
                        .request(Message::Run(Request::new(cmd_line.into())))
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
        match self.get_mut(room) {
            Some(mut r) if r.jukebox.is_none() => {
                let (tx, rx) = oneshot::channel();
                r.request(Message::Get(Request::new(cmd_line.into()), tx))
                    .await
                    .ok()?;
                rx.await.ok()
            }
            _ => None,
        }
    }

    pub fn create_jukebox(&self, name: RoomName) -> Jukebox {
        let mut r = Room::new(name.clone());
        let jb = r.jukebox.take().unwrap();
        self.rooms.insert(name, r);
        jb
    }

    #[inline(always)]
    pub fn get(&self, name: &RoomName) -> Option<Ref<'_, RoomName, Room>> {
        self.rooms.get(name)
    }

    #[inline(always)]
    pub fn get_mut(
        &self,
        name: &RoomName,
    ) -> Option<RefMut<'_, RoomName, Room>> {
        self.rooms.get_mut(name)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub id: usize,
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
    pub id: usize,
    pub response: Result<String, String>,
}

impl Response {
    pub fn new(r: Request, response: Result<String, String>) -> Self {
        Self { id: r.id, response }
    }
}

#[derive(Debug)]
pub enum Message {
    Run(Request),
    Get(Request, oneshot::Sender<Result<String, String>>),
    Reconnect(Arc<Notify>),
}

#[derive(Debug)]
pub struct Jukebox {
    name: RoomName,
    channel: Receiver<Message>,
}

impl Jukebox {
    fn new(name: RoomName, channel: Receiver<Message>) -> Self {
        Self { name, channel }
    }

    pub fn name(&self) -> &RoomName {
        &self.name
    }

    pub async fn recv(&mut self) -> Option<Message> {
        self.channel.recv().await
    }
}
