pub mod api;
pub mod export;
mod launcher;
pub mod parallel;
pub mod progress;

pub use launcher::{Instance, Launcher, Loader, LoaderKind, LoaderSpec};

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_digest_to_hex_is_40_lowercase_chars() {
        let d = Sha1Digest::from_bytes([0x12; 20]);
        assert_eq!(d.to_hex(), "12".repeat(20));
    }

    /// Piston and Modrinth responses embed digests as hex strings; if
    /// serde/hex ever diverge from the on-disk format the whole cache
    /// integrity check silently breaks.
    #[test]
    fn sha1_digest_json_roundtrip() {
        let bytes: [u8; 20] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f, 0x10, 0x11, 0x12, 0x13,
        ];
        let original = Sha1Digest::from_bytes(bytes);
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(json, "\"000102030405060708090a0b0c0d0e0f10111213\"");
        let restored: Sha1Digest = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }
}
