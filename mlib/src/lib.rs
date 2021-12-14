#![warn(clippy::dbg_macro)]

#[cfg(feature = "playlist")]
pub mod playlist;
#[cfg(feature = "socket")]
pub mod socket;
#[cfg(feature = "ytdl")]
pub mod ytdl;
