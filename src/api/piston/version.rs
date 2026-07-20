use serde::Deserialize;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    OldAlpha,
    OldBeta,
    Release,
    Snapshot,
}
