use std::sync::OnceLock;

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub fn client() -> reqwest::Result<&'static reqwest::Client> {
    match CLIENT.get() {
        Some(c) => Ok(c),
        None => {
            let client = reqwest::ClientBuilder::new()
                .user_agent(format!("mlib/{}", env!("CARGO_PKG_VERSION")))
                .build()?;
            Ok(CLIENT.get_or_init(|| client))
        }
    }
}
