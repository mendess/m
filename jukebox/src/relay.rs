use once_cell::sync::Lazy;
use std::{collections::HashMap, marker::Unpin, net::Ipv4Addr, sync::Mutex};
use tokio::{
    io::{self, AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
    prelude::*,
    stream::StreamExt,
    sync::mpsc::{self, Receiver, Sender},
};

static ROOMS: Lazy<Mutex<HashMap<String, Sender<String>>>> = Lazy::new(Mutex::default);

#[derive(Debug)]
enum Kind {
    Jukebox(Receiver<String>),
    User(Sender<String>),
}

async fn get_name<R, W, F>(mut reader: R, mut writer: W, check: F) -> io::Result<String>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
    F: Fn(&str) -> bool,
{
    let mut s = String::new();
    loop {
        s.clear();
        reader.read_line(&mut s).await?;
        eprintln!("Room name attempt: {:?}", s);
        if check(s.trim()) {
            break;
        }
        writer.write_all(b"invalid name").await?;
    }
    Ok(s)
}

async fn protocol<R, W>(mut reader: R, mut writer: W) -> io::Result<Kind>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut s = String::with_capacity(7);
    reader.read_line(&mut s).await?;
    let (kind, name) = match s.trim() {
        "user" => {
            let name = get_name(&mut reader, &mut writer, |s| {
                ROOMS.lock().unwrap().contains_key(s)
            })
            .await?;
            let rx = match ROOMS.lock().unwrap().get(&name) {
                Some(rx) => rx.clone(),
                None => todo!(),
            };
            Ok((Kind::User(rx), name))
        }
        "jukebox" => {
            let name = get_name(&mut reader, &mut writer, |s| {
                !ROOMS.lock().unwrap().contains_key(s)
            })
            .await?;
            let (rx, tx) = mpsc::channel(64);
            ROOMS.lock().unwrap().insert(name.clone(), rx);
            Ok((Kind::Jukebox(tx), name))
        }
        _ => Err(io::Error::new(io::ErrorKind::Other, "Invalid user kind")),
    }?;
    writer.write_all(b"accepted").await?;
    eprintln!("name: {}", name);
    Ok(kind)
}

async fn handle_user<R, W>(mut reader: R, mut writer: W, mut ch: Sender<String>) -> io::Result<()>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut s = String::new();
    while reader.read_line(&mut s).await? > 0 {
        s.pop();
        ch.send(s.clone()).await.expect("fix me");
    }
    Ok(())
}

async fn handle_jukebox<R, W>(mut reader: R, mut writer: W, ch: Receiver<String>) -> io::Result<()>
where
    R: AsyncBufReadExt + Unpin,
    W: AsyncWrite + Unpin,
{
    Ok(())
}

async fn handle(mut stream: TcpStream) -> io::Result<()> {
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);
    let k = protocol(&mut reader, &mut writer).await?;
    match k {
        Kind::User(ch) => handle_user(reader, writer, ch).await?,
        Kind::Jukebox(ch) => handle_jukebox(reader, writer, ch).await?,
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
