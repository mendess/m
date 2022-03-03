#![deny(missing_docs)]

use std::ops::Deref;
use std::path::PathBuf;
use std::str::FromStr;

use structopt::clap::AppSettings::DisableVersion;
use structopt::clap::Shell;
use structopt::StructOpt;

#[derive(Debug, StructOpt)]
pub struct Args {
    #[structopt(short, long)]
    pub socket: Option<usize>,
    #[structopt(flatten)]
    pub cmd: Command,
}

#[derive(Debug, StructOpt)]
pub enum Command {
    /// Toggle pause
    #[structopt(alias = "p")]
    Pause,

    /// Kill the most recent player
    Quit,

    /// Play something
    Play(Play),

    /// Interactively asks the user what songs they want to play from their playlist
    #[structopt(alias = "play-interactive")]
    Playlist,

    /// Add a new song to the playlist
    #[structopt(alias = "add-song")]
    New(New),

    /// Append a playlist to the personal playlist
    AddPlaylist(AddPlaylist),

    /// List all current categories
    Cat,

    /// Shows the current playlist
    Now(Amount),

    /// Show the current song
    #[structopt(alias = "c")]
    Current {
        /// With a notification
        #[structopt(short, long)]
        notify: bool,
        /// Print the filename/link instead
        #[structopt(short = "i", long)]
        link: bool,
    },

    /// Shows lyrics for the current song
    #[structopt(alias = "ly")]
    Lyrics,

    /// Add a category to the current song
    #[structopt(alias = "change-cats-to-current")]
    ChCat, // TODO: review this

    /// Queue a song
    #[structopt(alias = "q")]
    Queue(Queue),

    /// Dequeue a song
    #[structopt(alias = "dq")]
    Dequeue(DeQueue),

    /// Delete a song from the playlist file
    #[structopt(alias = "del")]
    DeleteSong(DeleteSong),

    /// Deletes downloaded songs that are not in the playlist anymore
    CleanDownloads,

    /// Toggles playlist looping
    Loop,

    /// Volume up
    #[structopt(alias = "k")]
    Vu(Amount),

    /// Volume up
    #[structopt(alias = "j")]
    Vd(Amount),

    /// Previous chapter in a file
    #[structopt(alias = "H")]
    Prev(Amount),

    /// Next chapter in a file
    #[structopt(alias = "L")]
    Next(Amount),

    /// Previous file
    #[structopt(alias = "h")]
    PrevFile(Amount),

    /// Skip to the next file
    #[structopt(alias = "l")]
    NextFile(Amount),

    /// Seek backward
    #[structopt(alias = "u", alias = "J")]
    Back(Amount),

    /// Seek forward
    #[structopt(alias = "i", alias = "K")]
    Frwd(Amount),

    /// Enter interactive mode
    #[structopt(alias = "int")]
    Interactive,

    // TODO: jukebox? probably deprecated
    /// Toggle video
    ToggleVideo,

    /// Get all songs in the playlist, optionaly filtered by category
    Songs { category: Option<String> },

    /// Save the playlist to a file to be restored later
    Dump { file: PathBuf },

    /// Load a file of songs to play
    Load { file: PathBuf },

    /// Get the socket in use
    Socket {
        #[structopt(parse(try_from_str = parse_new))]
        new: Option<()>, // yes, very much hack
    },

    /// Shuffle
    #[structopt(alias = "shuf")]
    Shuffle,

    /// Status
    Status {
        #[structopt(default_value = "players")]
        entity: EntityStatus,
    },

    /// Info
    Info { song: Vec<String> },

    /// Generate auto complete script
    AutoComplete { shell: Shell },
}

fn parse_new(s: &str) -> Result<(), &'static str> {
    if s == "new" {
        Ok(())
    } else {
        Err("only 'new' can be passed to socket")
    }
}

#[derive(Debug, StructOpt)]
#[structopt(global_settings = &[DisableVersion])]
pub struct Play {
    /// Search the song on youtube
    #[structopt(short, long)]
    pub search: bool,
    pub what: Vec<String>,
}

#[derive(Debug, StructOpt)]
#[structopt(global_settings = &[DisableVersion])]
pub struct New {
    /// Queue it too
    #[structopt(short, long)]
    pub queue: bool,
    #[structopt(short, long)]
    pub search: bool,
    pub query: String,
    pub categories: Vec<String>,
}

#[derive(Debug, StructOpt)]
#[structopt(global_settings = &[DisableVersion])]
pub struct AddPlaylist {
    /// Queue it too
    #[structopt(short, long)]
    pub queue: bool,
    pub link: String,
    pub categories: Vec<String>,
}

#[derive(Debug, StructOpt)]
#[structopt(global_settings = &[DisableVersion])]
pub struct Queue {
    #[structopt(flatten)]
    pub queue_opts: QueueOpts,

    #[structopt(flatten)]
    pub play_opts: Play,
}

#[derive(Debug, StructOpt, Default)]
#[structopt(global_settings = &[DisableVersion])]
pub struct QueueOpts {
    /// Resets the queue fairness
    #[structopt(short, long)]
    pub reset: bool,

    /// Send a notification
    #[structopt(short, long)]
    pub notify: bool,

    /// Don't move in the playlist, keep it at the end
    #[structopt(short = "m", long = "no-move")]
    pub no_move: bool,

    /// Clear the queue
    #[structopt(short = "x", long = "clear")]
    pub clear: bool,

    /// Queue all songs in a category
    #[structopt(short, long)]
    pub category: Option<String>,

    /// Don't preemptively download songs
    #[structopt(short = "p", long = "no-preempt-download")]
    pub preemptive_download: bool,
}

impl Deref for Queue {
    type Target = Play;
    fn deref(&self) -> &Self::Target {
        &self.play_opts
    }
}

#[derive(Debug, StructOpt)]
#[structopt(global_settings = &[DisableVersion])]
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

#[derive(Debug)]
pub enum DeQueueIndexKind {
    Minus,
    Plus,
    Exact,
}

#[derive(Debug)]
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

#[derive(Debug, StructOpt)]
#[structopt(global_settings = &[DisableVersion])]
pub struct DeleteSong {
    #[structopt(short, long)]
    pub current: bool,
    pub partial_name: Vec<String>, // TODO: incompatible with current
}

#[derive(Debug, StructOpt)]
#[structopt(global_settings = &[DisableVersion])]
pub struct Amount {
    pub amount: Option<isize>,
}

#[derive(Debug, StructOpt)]
#[structopt(global_settings = &[DisableVersion])]
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
