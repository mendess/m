mod prompt;
mod relay;
mod server;

use once_cell::sync::Lazy;
use std::{
    fmt::{self, Display},
    io,
    net::TcpStream,
    str::FromStr,
};
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
#[structopt(name = "jukebox")]
struct Opt {
    /// Select running mode
    ///
    /// Modes:
    ///  - server: listens for commands to run on the local player
    ///  - relay: receives a command and relays it to the registered player
    ///  - jukebox: connect to a relay and serve as jukebox
    ///  - user: connect to a relay to remote control a jukebox
    #[structopt(subcommand)]
    mode: Mode,
    /// Port to use for server or relay
    #[structopt(default_value = "4192", short, long)]
    port: u16,
    /// Endpoint to use
    #[structopt(default_value = "mendess.xyz", short, long)]
    endpoint: String,
}

static OPTIONS: Lazy<Opt> = Lazy::new(Opt::from_args);

#[cfg(not(target_os = "android"))]
fn connect_to_relay(port: u16) -> io::Result<TcpStream> {
    for ip in dns_lookup::lookup_host(&OPTIONS.endpoint)? {
        match TcpStream::connect((ip, port)) {
            Ok(s) => return Ok(s),
            Err(_) => eprintln!("Failed to connect to {}", ip),
        }
    }
    Err(io::ErrorKind::ConnectionRefused.into())
}

#[cfg(target_os = "android")]
fn connect_to_relay(port: u16) -> io::Result<TcpStream> {
    use std::{net::IpAddr, process::Command};
    let o = Command::new("ping")
        .args(&["-c", "1", &OPTIONS.endpoint])
        .output()?;
    let stdout = std::str::from_utf8(&o.stdout)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let start_ip = stdout.find('(');
    let end_ip = stdout.find(')');
    if let (Some(start_ip), Some(end_ip)) = (start_ip, end_ip) {
        let ip = stdout[(start_ip + 1)..end_ip]
            .parse::<IpAddr>()
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
        TcpStream::connect((ip, port))
    } else {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Could not resolve endpoint",
        ))
    }
}

fn print_result<S: Display>(r: &Result<S, S>) {
    match r {
        Ok(s) => println!("{}", s),
        Err(e) => println!("\x1b[1;31mError:\x1b[0m\n{}", e),
    }
}

#[derive(Debug, StructOpt)]
enum Mode {
    Server,
    Relay,
    Jukebox,
    User,
    Admin,
}

impl FromStr for Mode {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "server" => Ok(Self::Server),
            "relay" => Ok(Self::Relay),
            "jukebox" => Ok(Self::Jukebox),
            "user" => Ok(Self::User),
            "admin" => Ok(Self::Admin),
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
            Self::User => "user",
            Self::Admin => "admin",
        };
        write!(f, "{}", s)
    }
}

#[tokio::main]
async fn main() {
    let r = match OPTIONS.mode {
        Mode::Server => server::run(OPTIONS.port).await,
        Mode::Relay => relay::run(OPTIONS.port).await,
        Mode::Jukebox => relay::jukebox::run(OPTIONS.port),
        Mode::User => relay::user::run(OPTIONS.port),
        Mode::Admin => relay::admin::run(OPTIONS.port),
    };
    if let Err(e) = r {
        eprintln!("Terminating because: {}", e);
    }
}
