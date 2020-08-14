mod relay;
mod server;

use std::{
    fmt::{self, Display},
    str::FromStr,
};
use structopt::StructOpt;

#[derive(Debug)]
enum Mode {
    Server,
    Relay,
}

impl FromStr for Mode {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "server" => Ok(Self::Server),
            "relay" => Ok(Self::Relay),
            _ => Err("Invalid running mode"),
        }
    }
}

impl Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Server => "server",
            Self::Relay => "relay",
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
    };
    if let Err(e) = r {
        eprintln!("Server stopped with error: {}", e);
    }
}
