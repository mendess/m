pub mod admin;
pub mod jukebox;
pub mod user;
pub mod client_util;
pub mod server;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    id: usize,
    s: String,
}

impl Request {
    pub fn command(&self) -> Vec<&str> {
        crate::arg_split::quoted_parse(&self.s)
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    id: usize,
    data: Result<String, String>,
}

impl Response {
    pub fn new(r: Request, data: Result<String, String>) -> Self {
        Self { id: r.id, data }
    }
}


