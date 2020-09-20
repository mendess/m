use jukebox::{
    relay,
    relay_clients::{jbox, user::Client},
    server, RoomName,
};
use std::{
    fmt::{self, Display},
    str::FromStr,
    time::Duration,
};
use structopt::StructOpt;

fn parse_duration(s: &str) -> Result<Duration, String> {
    match s.chars().last() {
        Some(c) if c.len_utf8() == 1 => {
            let n = s[..(s.len() - 1)]
                .parse::<u64>()
                .map_err(|_| "invalid digit".to_string())?;
            match c {
                's' => Ok(Duration::from_secs(n)),
                'm' => Ok(Duration::from_millis(n)),
                _ => Err(format!("invalid format '{}'", c)),
            }
        }
        None => Err("empty duration".into()),
        Some(c) => Err(format!("invalid format '{}'", c)),
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "jukebox")]
struct Opt {
    /// Select running mode
    ///
    /// Modes:
    #[structopt(subcommand)]
    mode: Mode,
    /// Port to use for server or relay
    #[structopt(default_value = "4192", short, long)]
    port: u16,
    /// Endpoint to use
    #[structopt(default_value = "mendess.xyz", short, long)]
    endpoint: String,
    /// Reconnect timeout
    #[structopt(parse(try_from_str = parse_duration), default_value = "5s", short, long)]
    reconnect: Duration,
    /// Room name
    #[structopt(short("n"), long)]
    room: Option<RoomName>,
}

#[derive(Debug, StructOpt)]
enum Mode {
    /// listens for commands to run on the local player
    Server,
    /// receives a command and relays it to the registered player
    Relay,
    /// connect to a relay and serve as jukebox
    Jukebox,
    /// connect to a relay to remote control a jukebox
    User,
}

impl FromStr for Mode {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "server" => Ok(Self::Server),
            "relay" => Ok(Self::Relay),
            "jukebox" => Ok(Self::Jukebox),
            "user" => Ok(Self::User),
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
        };
        write!(f, "{}", s)
    }
}

#[tokio::main]
async fn main() -> Result<(), tokio::io::Error> {
    let options = Opt::from_args();
    match options.mode {
        Mode::Server => server::run(options.port).await,
        Mode::Relay => relay::start(options.port).await,
        Mode::Jukebox => {
            let addr = (options.endpoint.as_str(), options.port);
            match options.room {
                Some(r) => jbox::with_room_name(addr, options.reconnect, r),
                None => jbox::run(addr, options.reconnect),
            }
        }
        Mode::User => {
            let cli = match options.room {
                Some(r) => {
                    Client::with_room_name(&options.endpoint, options.port, r)
                }
                None => Client::new(&options.endpoint, options.port),
            }
            .unwrap();
            cli.run().await
        }
    }
}
