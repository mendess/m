use jukebox::{
    relay::{self, client_util},
    server,
};
use std::{
    fmt::{self, Display},
    io,
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
    #[structopt(short, long)]
    room: Option<String>,
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
    let options = Opt::from_args();
    let addr = (options.endpoint.as_str(), options.port);
    let prompt = jukebox::prompt::Prompt::default();
    let r = match options.mode {
        Mode::Server => server::run(options.port).await,
        Mode::Relay => relay::server::run(options.port).await,
        Mode::Jukebox => match options.room {
            Some(r) => relay::jukebox::start_protocol(addr, options.reconnect)
                .and_then(|(mut socket, room_name)| {
                    if client_util::attempt_room_name(&mut socket, &r)? {
                        *room_name.borrow_mut() = r;
                        Ok(socket)
                    } else {
                        Err(io::Error::new(
                            io::ErrorKind::Other,
                            "room name taken",
                        ))
                    }
                })
                .and_then(relay::jukebox::execute_loop),
            None => relay::jukebox::run(addr, options.reconnect),
        },
        Mode::User => relay::user::run(addr, options.reconnect, prompt),
        Mode::Admin => relay::admin::run(addr),
    };
    if let Err(e) = r {
        eprintln!("Terminating because: {}", e);
    }
}
