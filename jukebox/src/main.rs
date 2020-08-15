mod jukebox;
mod prompt;
mod relay;
mod server;
mod user;

use dns_lookup::getaddrinfo;
use std::{
    fmt::{self, Display},
    io,
    net::TcpStream,
    str::FromStr,
};
use structopt::StructOpt;

fn connect_to_relay(port: u16) -> io::Result<TcpStream> {
    let sockets =
        getaddrinfo(Some("mendess.xyz"), None, None)?.collect::<std::io::Result<Vec<_>>>()?;

    for socket in sockets {
        // Try connecting to socket
        match TcpStream::connect((socket.sockaddr.ip(), port)) {
            Ok(s) => return Ok(s),
            Err(_) => eprintln!("Failed to connect to {}", socket.sockaddr),
        }
    }
    Err(io::ErrorKind::ConnectionRefused.into())
}

#[derive(Debug)]
enum Mode {
    Server,
    Relay,
    Jukebox,
    RelayUser,
}

impl FromStr for Mode {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "server" => Ok(Self::Server),
            "relay" => Ok(Self::Relay),
            "jukebox" => Ok(Self::Jukebox),
            "user" => Ok(Self::RelayUser),
            _ => Err("Invalid running mode"),
        }
    }
}

impl Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Server => "server",
            Self::Relay => "relay",
            Self::Jukebox => "jukebox",
            Self::RelayUser => "user",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "jukebox")]
struct Opt {
    /// Select running mode
    ///
    /// Modes:
    ///  - server: listens for commands to run on the local player
    ///  - relay: receives a command and relays it to the registered player
    ///  - jukebox: connect to a relay and serve as jukebox
    #[structopt(short, long)]
    mode: Mode,
    /// Port to use for server or relay
    #[structopt(default_value = "4192", short, long)]
    port: u16,
}

#[tokio::main]
async fn main() {
    let options = Opt::from_args();
    let r = match options.mode {
        Mode::Server => server::run(options.port).await,
        Mode::Relay => relay::run(options.port).await,
        Mode::Jukebox => jukebox::run(options.port),
        Mode::RelayUser => user::run(options.port),
    };
    if let Err(e) = r {
        eprintln!("Terminating because: {}", e);
    }
}
