use crate::{
    prompt::{pretty_prompt, Prompt},
    relay::web_server::Req,
    try_prompt, RoomName, Ui,
};
use reqwest::{Client as RClient, Url};
use tokio::io;
use itertools::Itertools;

pub struct Client {
    room_name: Option<RoomName>,
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
        Ok(Self {
            room_name: room_name.into(),
            base_url: Url::parse(&format!(
                "http://{}:{}/",
                endpoint,
                port + 1
            ))?,
            prompt: pretty_prompt(),
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

    async fn inner_run(mut self) -> Result<(), Error> {
        let Self {
            room_name,
            base_url,
            prompt,
            requests,
        } = &mut self;
        let mut rn = match room_name.take() {
            Some(n) => n,
            None => {
                let r =
                    requests.get(base_url.join("rooms").unwrap()).send().await;
                match r {
                    Ok(rooms) => prompt.inform(format!(
                        "Available rooms: {}",
                        rooms
                            .json::<Vec<(String, bool)>>()
                            .await?
                            .into_iter()
                            .map(|(name, state)| format!(
                                "{} {}",
                                if state { "ONLINE " } else { "OFFLINE" },
                                name
                            ))
                            .join("\n")
                    )),
                    Err(e) => {
                        prompt.inform::<&Result<&str, _>>(&Err(&e));
                        return Err(e.into());
                    }
                }
                try_prompt!(prompt.room_name())
            }
        };
        rn.name.push('/');
        let gets = base_url.join("get/").unwrap().join(&rn.name).unwrap();
        let runs = base_url.join("run/").unwrap().join(&rn.name).unwrap();
        loop {
            let cmd = try_prompt!(prompt.command());
            let url = match method_picker(&cmd) {
                Some(Method::Get) => &gets,
                Some(Method::Run) => &runs,
                None => {
                    prompt.inform::<Result<&str, _>>(Err("Invalid command"));
                    continue;
                }
            };
            match requests.get(url.clone()).json(&Req::from(cmd)).send().await {
                Ok(r) => prompt.inform(r.text().await),
                Err(e) => prompt.inform(
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
