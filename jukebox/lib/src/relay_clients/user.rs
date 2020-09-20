use crate::{
    prompt::{pretty_prompt, Prompt},
    relay::web_server::Req,
    try_prompt, RoomName,
};
use itertools::Itertools;
use reqwest::{Client as RClient, Url};
use tokio::io;

pub struct Client {
    base_url: Url,
    prompt: Prompt,
    requests: RClient,
}

impl Client {
    pub fn new(endpoint: &str, port: u16) -> Result<Self, url::ParseError> {
        Self::with_room_name(endpoint, port, None)
    }

    pub fn with_room_name<I: Into<Option<RoomName>>>(
        endpoint: &str,
        port: u16,
        room_name: I,
    ) -> Result<Self, url::ParseError> {
        let room_name = room_name.into();
        Ok(Self {
            base_url: Url::parse(&format!(
                "http://{}:{}/",
                endpoint,
                port + 1
            ))?,
            prompt: pretty_prompt().with_room_name(room_name),
            requests: Default::default(),
        })
    }

    pub async fn run(self) -> io::Result<()> {
        self.inner_run().await.map_err(|e| match e {
            Error::Io(e) => e,
            Error::Reqwest(e) => {
                io::Error::new(io::ErrorKind::Other, e.to_string())
            }
        })
    }

    async fn change_room_name(&mut self) -> Result<(), Error> {
        match self
            .requests
            .get(self.base_url.join("rooms").unwrap())
            .send()
            .await
        {
            Ok(rooms) => {
                let rooms = rooms.json::<Vec<(String, bool)>>().await?;
                if rooms.is_empty() {
                    self.prompt.inform("No rooms available")
                } else {
                    self.prompt.inform(format!(
                        "\x1b[1mAvailable rooms:\x1b[0m\n{}",
                        rooms
                            .into_iter()
                            .map(|(name, state)| format!(
                                "{}\x1b[0m {}",
                                if state {
                                    "\x1b[32mONLINE "
                                } else {
                                    "\x1b[31mOFFLINE"
                                },
                                name
                            ))
                            .join("\n")
                    ))
                }
            }
            Err(e) => {
                self.prompt.inform::<&Result<&str, _>>(&Err(&e));
                return Err(e.into());
            }
        }
        try_prompt!(self.prompt.ask_room_name());
        Ok(())
    }

    async fn inner_run(mut self) -> Result<(), Error> {
        if self.prompt.room_name().is_none() {
            self.change_room_name().await?;
        }
        let gets = self
            .base_url
            .join("get/")
            .unwrap()
            .join(&self.prompt.room_name().unwrap().name)
            .unwrap();
        let runs = self
            .base_url
            .join("run/")
            .unwrap()
            .join(&self.prompt.room_name().unwrap().name)
            .unwrap();
        loop {
            let cmd = try_prompt!(self.prompt.command());
            let url = match method_picker(&cmd) {
                Some(Method::Get) => &gets,
                Some(Method::Run) => &runs,
                Some(Method::Rooms) => {
                    self.change_room_name().await?;
                    continue;
                }
                None => {
                    self.prompt
                        .inform::<Result<&str, _>>(Err("Invalid command"));
                    continue;
                }
            };
            match self
                .requests
                .post(url.clone())
                .json(&Req::from(cmd))
                .send()
                .await
            {
                Ok(r) => self.prompt.inform(r.text().await),
                Err(e) => self.prompt.inform(
                    e.status()
                        .map(|s| s.as_str().to_string())
                        .ok_or_else(|| e.to_string()),
                ),
            }
        }
    }
}

enum Method {
    Get,
    Run,
    Rooms,
}

fn method_picker(cmd: &str) -> Option<Method> {
    match cmd.trim().split_whitespace().next().unwrap() {
        "p" | "pause" => Some(Method::Run),
        "cat" => Some(Method::Get),
        "now" => Some(Method::Get),
        "c" | "current" => Some(Method::Get),
        "q" | "queue" => Some(Method::Run),
        "loop" => Some(Method::Run),
        "k" | "vu" => Some(Method::Run),
        "j" | "vd" => Some(Method::Run),
        "h" | "prev-file" => Some(Method::Run),
        "l" | "next-file" => Some(Method::Run),
        "J" | "back" => Some(Method::Run),
        "K" | "frwd" => Some(Method::Run),
        ":rooms" => Some(Method::Rooms),
        _ => None,
    }
}

enum Error {
    Io(io::Error),
    Reqwest(reqwest::Error),
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<reqwest::Error> for Error {
    fn from(e: reqwest::Error) -> Self {
        Self::Reqwest(e)
    }
}
