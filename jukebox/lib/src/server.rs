use std::net::Ipv4Addr;
use tokio::{
    io::{self, BufReader},
    net::{TcpListener, TcpStream},
    prelude::*,
    process::Command,
    stream::StreamExt,
};

async fn handle(mut stream: TcpStream) -> io::Result<()> {
    let (reader, mut writer) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut s = String::new();
    while reader.read_line(&mut s).await? > 0 {
        let args = crate::arg_split::quoted_parse(&s);
        eprintln!("Running command: {:?}", args);
        let o = Command::new("m").args(&args).output().await?;

        let mut response = String::from_utf8(o.stdout).map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "invalid utf-8")
        })?;
        response += std::str::from_utf8(&o.stderr).map_err(|_| {
            io::Error::new(io::ErrorKind::Other, "invalid utf-8")
        })?;

        let r = if o.status.success() {
            Ok(response)
        } else {
            Err(response)
        };

        writer
            .write_all(serde_json::to_string(&r)?.as_bytes())
            .await?;
        s.clear();
    }
    Ok(())
}

pub async fn run(port: u16) -> io::Result<()> {
    println!("Serving on port: {}", port);
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
