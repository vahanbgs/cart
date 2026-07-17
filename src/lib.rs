mod cli;
mod config;
mod launcher;
pub mod piston;

pub use cli::Cli;
pub use config::CartManifest;
pub use launcher::Launcher;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sha1Digest(#[serde(with = "hex::serde")] [u8; 20]);

impl Sha1Digest {
    pub fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}
