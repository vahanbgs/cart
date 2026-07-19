use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ModDependency {
    Url { url: Url },
}
