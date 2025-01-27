use std::ops::Deref;
use std::path::PathBuf;
use std::str::FromStr;

use clap::{Parser, Subcommand};
use clap_complete::Shell;
use serde::{Deserialize, Serialize};

#[derive(Debug, Parser, Serialize, Deserialize)]
#[command(author, version, about, long_about = None)]
pub struct Args {
    #[arg(short, long)]
    pub socket: Option<usize>,
    #[command(subcommand)]
    pub cmd: Option<Command>,
}

#[derive(Debug, Clone, Subcommand, Serialize, Deserialize)]
pub enum Command {
    SetPlay,
    SetPause,
    /// Toggle pause
    #[command(alias = "p")]
    Pause,

    /// Kill the most recent player
    Quit,

    /// Play something
    Play(Play),

    /// Interactively asks the user what songs they want to play from their playlist
    #[command(alias = "play-interactive")]
    Playlist,

    /// Add a new song to the playlist
    #[command(alias = "add-song")]
    New(New),

    /// Append a playlist to the personal playlist
    AddPlaylist(AddPlaylist),

    /// List all current categories
    Cat,

    /// Shows the current playlist
    Now(Amount),

    /// Show the current song
    #[command(alias = "c")]
    Current {
        /// With a notification
        #[arg(short, long)]
        notify: bool,
        /// Print the filename/link instead
        #[arg(short = 'i', long, action = clap::ArgAction::Count)]
        link: u8,
    },

    /// Shows lyrics for the current song
    #[command(alias = "ly")]
    Lyrics,

    /// Add a category to the current song
    #[command(alias = "change-cats-to-current")]
    ChCat, // TODO: review this

    /// Queue a song
    #[command(alias = "q")]
    Queue(Queue),

    /// Dequeue a song
    #[command(subcommand, alias = "dq")]
    Dequeue(DeQueue),

    /// Delete a song from the playlist file
    #[command(alias = "del")]
    DeleteSong(DeleteSong),

    /// Deletes downloaded songs that are not in the playlist anymore
    CleanDownloads,

    /// Toggles playlist looping
    Loop,

    /// Volume up
    #[command(alias = "k")]
    Vu(Amount),

    /// Volume up
    #[command(alias = "j")]
    Vd(Amount),

    /// Previous chapter in a file
    #[command(alias = "H")]
    Prev(Amount),

    /// Next chapter in a file
    #[command(alias = "L")]
    Next(Amount),

    /// Previous file
    #[command(alias = "h")]
    PrevFile(Amount),

    /// Skip to the next file
    #[command(alias = "l")]
    NextFile(Amount),

    /// Seek backward
    #[command(alias = "u", alias = "J")]
    Back(Amount),

    /// Seek forward
    #[command(alias = "i", alias = "K")]
    Frwd(Amount),

    /// Enter interactive mode
    #[command(alias = "int")]
    Interactive,

    // TODO: jukebox? probably deprecated
    /// Toggle video
    ToggleVideo,

    /// Get all songs in the playlist, optionaly filtered by category
    Songs {
        category: Option<String>,
    },

    /// Save the playlist to a file to be restored later
    Dump {
        file: PathBuf,
    },

    /// Load a file of songs to play
    Load {
        file: PathBuf,
        #[arg(short, long)]
        shuf: bool,
    },

    /// Get the socket in use
    Socket {
        #[arg(value_parser = parse_new, id = "new")]
        new: Option<()>, // yes, very much hack
    },

    /// Shuffle
    #[command(alias = "shuf")]
    Shuffle,

    /// Status
    Status {
        #[arg(default_value = "players")]
        entity: EntityStatus,
    },

    /// Info
    Info {
        #[arg(short, long)]
        id: bool,
        song: Vec<String>,
    },

    /// Generate auto complete script
    #[serde(skip)]
    AutoComplete {
        shell: Shell,
    },

    /// Just download the missing songs
    Download {
        category: Option<String>,
        what: Option<Vec<String>>,
    },
}

fn parse_new(s: &str) -> Result<(), &'static str> {
    if s == "new" {
        Ok(())
    } else {
        Err("only 'new' can be passed to socket")
    }
}

#[derive(Debug, Clone, Parser, Serialize, Deserialize)]
// #[structopt(global_settings = &[DisableVersion])]
pub struct Play {
    /// Search the song on youtube
    #[arg(short, long)]
    pub search: bool,

    /// Whether to enable video or not
    #[arg(short, long)]
    pub video: bool,

    /// Queue all songs in a category
    #[arg(short, long)]
    pub category: Option<String>,

    /// What to play
    pub what: Vec<String>,
}

#[derive(Debug, Clone, Parser, Serialize, Deserialize)]
// #[structopt(global_settings = &[DisableVersion])]
pub struct New {
    /// Queue it too
    #[arg(short, long)]
    pub queue: bool,
    #[arg(short, long)]
    pub search: bool,
    pub query: String,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Parser, Serialize, Deserialize)]
// #[structopt(global_settings = &[DisableVersion])]
pub struct AddPlaylist {
    /// Queue it too
    #[arg(short, long)]
    pub queue: bool,
    pub link: String,
    pub categories: Vec<String>,
}

#[derive(Debug, Clone, Parser, Serialize, Deserialize)]
// #[structopt(global_settings = &[DisableVersion])]
pub struct Queue {
    #[command(flatten)]
    pub queue_opts: QueueOpts,

    #[command(flatten)]
    pub play_opts: Play,
}

#[derive(Debug, Clone, Parser, Default, Serialize, Deserialize)]
// #[structopt(global_settings = &[DisableVersion])]
pub struct QueueOpts {
    /// Resets the queue fairness
    #[arg(short, long)]
    pub reset: bool,

    /// Send a notification
    #[arg(short, long)]
    pub notify: bool,

    /// Don't move in the playlist, keep it at the end
    #[arg(short = 'm', long = "no-move")]
    pub no_move: bool,

    /// Clear the queue
    #[arg(short = 'x', long = "clear")]
    pub clear: bool,
}

impl Deref for Queue {
    type Target = Play;
    fn deref(&self) -> &Self::Target {
        &self.play_opts
    }
}

#[derive(Debug, Clone, Subcommand, Serialize, Deserialize)]
// #[structopt(global_settings = &[DisableVersion])]
pub enum DeQueue {
    /// The next song in the queue
    Next,
    /// The previous song in the queue
    Prev,
    /// The song that was just added
    Pop,
    /// All songs that belong to a category
    Cat {
        /// The category to filter by
        cat: String,
    },
    /// The current song.
    Current,
    /// A relative index
    N {
        /// -X is X songs before the current one
        ///
        /// +X is X songs after the current one
        ///
        /// X is the song at position X in the queue
        i: DeQueueIndex,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum DeQueueIndexKind {
    Minus,
    Plus,
    Exact,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DeQueueIndex(pub DeQueueIndexKind, pub usize);

impl FromStr for DeQueueIndex {
    type Err = &'static str;

    fn from_str(mut s: &str) -> Result<Self, Self::Err> {
        let mut iter = s.chars();
        let fst = iter.next().ok_or("Empty string")?;
        let kind = match fst {
            '-' => {
                s = iter.as_str();
                DeQueueIndexKind::Minus
            }
            '+' => {
                s = iter.as_str();
                DeQueueIndexKind::Plus
            }
            _ => DeQueueIndexKind::Exact,
        };
        Ok(DeQueueIndex(kind, s.parse().map_err(|_| "invalid digit")?))
    }
}

#[derive(Debug, Clone, Parser, Serialize, Deserialize)]
// #[structopt(global_settings = &[DisableVersion])]
pub struct DeleteSong {
    #[arg(short, long)]
    pub current: bool,
    pub partial_name: Vec<String>, // TODO: incompatible with current
}

#[derive(Debug, Clone, Copy, Parser, Serialize, Deserialize)]
// #[structopt(global_settings = &[DisableVersion])]
pub struct Amount {
    pub amount: Option<i32>,
}

impl From<i32> for Amount {
    fn from(value: i32) -> Self {
        Self {
            amount: Some(value),
        }
    }
}

#[derive(Debug, Clone, Copy, Parser, Serialize, Deserialize)]
// #[structopt(global_settings = &[DisableVersion])]
pub enum EntityStatus {
    Players,
    Cache,
    Downloads,
}

impl FromStr for EntityStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match &s.to_lowercase()[..] {
            "players" => Ok(Self::Players),
            "cache" => Ok(Self::Cache),
            "downloads" => Ok(Self::Downloads),
            _ => Err(format!("Invalid entity: {}", s)),
        }
    }
}
