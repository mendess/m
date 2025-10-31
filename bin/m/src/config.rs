use std::{path::PathBuf, sync::LazyLock};

use dirs::config_dir;

#[derive(serde::Deserialize, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum DownloadFormat {
    #[default]
    Video,
    Audio,
}

#[derive(serde::Deserialize, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MConfig {
    #[serde(default)]
    pub socket_base_dir: Option<PathBuf>,
    #[serde(default)]
    pub download_format: DownloadFormat,
}

pub static CONFIG: LazyLock<MConfig> = LazyLock::new(|| {
    let mut config = config::Config::builder()
        .add_source({
            let mut base = config_dir().unwrap_or_else(|| {
                let mut home = dirs::home_dir().expect("can't find config dir or home dir");
                home.push(".config");
                home
            });
            base.push("m");
            base.push("config");
            config::File::from(base).required(false)
        })
        .build()
        .expect("a valid config file")
        .try_deserialize::<MConfig>()
        .expect("a valid config file");
    if let Some(base) = config.socket_base_dir.as_mut() {
        *base = base
            .iter()
            .map(|p| {
                if p == "~" {
                    dirs::home_dir()
                        .expect("can't find home dir")
                        .into_os_string()
                } else {
                    p.to_owned()
                }
            })
            .collect()
    }
    config
});
