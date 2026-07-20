pub mod piston;

use url::Url;

pub trait Endpoint: serde::de::DeserializeOwned {
    fn url() -> &'static Url;
}
