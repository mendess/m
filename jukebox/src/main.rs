mod prompt;
mod reconnect;
mod relay;
mod server;

use std::{
    fmt::{self, Display},
    str::FromStr,
    time::Duration,
};
use structopt::StructOpt;

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
}

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

fn print_result<S: Display>(r: &Result<S, S>) {
    match r {
        Ok(s) => println!("{}", s),
        Err(e) => println!("\x1b[1;31mError:\x1b[0m\n{}", e),
    }
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
    let r = match options.mode {
        Mode::Server => server::run(options.port).await,
        Mode::Relay => relay::run(options.port).await,
        Mode::Jukebox => relay::jukebox::run(addr, options.reconnect),
        Mode::User => relay::user::run(addr, options.reconnect),
        Mode::Admin => relay::admin::run(addr),
    };
    if let Err(e) = r {
        eprintln!("Terminating because: {}", e);
    }
}
