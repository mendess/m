pub mod m;
pub mod relay;
pub mod server;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Response(String);
