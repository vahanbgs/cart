use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
pub struct Rule {
    pub action: Action,
    pub os: Option<Os>,
    pub features: Option<Features>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Disallow,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Os {
    Arch { arch: Arch },
    Name { name: OsName },
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    X86,
}

#[derive(Clone, Copy, Debug, Deserialize, Hash, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OsName {
    Linux,
    Osx,
    Windows,
}

impl OsName {
    pub fn matches_current_platform(self) -> bool {
        match self {
            OsName::Linux => cfg!(target_os = "linux"),
            OsName::Osx => cfg!(target_os = "macos"),
            OsName::Windows => cfg!(target_os = "windows"),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Features {
    pub is_demo_user: Option<bool>,
    pub is_quick_play_realms: Option<bool>,
    pub is_quick_play_singleplayer: Option<bool>,
    pub is_quick_play_multiplayer: Option<bool>,
    pub has_custom_resolution: Option<bool>,
    pub has_quick_plays_support: Option<bool>,
}
