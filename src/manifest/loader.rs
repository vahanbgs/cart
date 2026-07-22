// Wired into `Manifest` in the next commit; keep the linter quiet meanwhile.
#![allow(dead_code)]

//! Mod-loader selector in `cart.toml`.
//!
//! ```toml
//! # loader = "fabric"                     # → Fabric latest
//! # loader = "forge"                      # → Forge latest
//! # loader = { fabric = "0.15.7" }        # pinned Fabric loader
//! # loader = { forge = "recommended" }    # Forge's stable channel
//! # loader = { forge = "47.3.12" }        # pinned Forge build
//! ```

use serde::{Deserialize, Deserializer};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Fabric,
    Forge,
}

/// Version specifier for a loader. `Recommended` is Forge-only; Fabric has no
/// analogous channel and rejecting it at parse time is cleaner than silently
/// aliasing it to `Latest`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoaderSpec {
    Latest,
    Recommended,
    Pinned(String),
}

impl LoaderSpec {
    fn parse(s: &str) -> Self {
        match s {
            "latest" => Self::Latest,
            "recommended" => Self::Recommended,
            other => Self::Pinned(other.to_owned()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Loader {
    pub kind: LoaderKind,
    pub spec: LoaderSpec,
}

impl<'de> Deserialize<'de> for Loader {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Bare(LoaderKind),
            Forge { forge: String },
            Fabric { fabric: String },
        }

        let repr = Repr::deserialize(deserializer)?;
        let loader = match repr {
            Repr::Bare(kind) => Loader {
                kind,
                spec: LoaderSpec::Latest,
            },
            Repr::Forge { forge } => Loader {
                kind: LoaderKind::Forge,
                spec: LoaderSpec::parse(&forge),
            },
            Repr::Fabric { fabric } => {
                let spec = LoaderSpec::parse(&fabric);
                if matches!(spec, LoaderSpec::Recommended) {
                    return Err(serde::de::Error::custom(
                        "`recommended` is a Forge-only channel; Fabric only has `latest` or a pinned version",
                    ));
                }
                Loader {
                    kind: LoaderKind::Fabric,
                    spec,
                }
            }
        };
        Ok(loader)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_body: &str) -> Loader {
        #[derive(Deserialize)]
        struct Wrapper {
            loader: Loader,
        }
        toml::from_str::<Wrapper>(toml_body).unwrap().loader
    }

    fn parse_err(toml_body: &str) -> String {
        #[derive(Debug, Deserialize)]
        struct Wrapper {
            #[allow(dead_code)]
            loader: Loader,
        }
        toml::from_str::<Wrapper>(toml_body).unwrap_err().to_string()
    }

    #[test]
    fn bare_string_forge_is_latest() {
        assert_eq!(
            parse(r#"loader = "forge""#),
            Loader {
                kind: LoaderKind::Forge,
                spec: LoaderSpec::Latest,
            }
        );
    }

    #[test]
    fn bare_string_fabric_is_latest() {
        assert_eq!(
            parse(r#"loader = "fabric""#),
            Loader {
                kind: LoaderKind::Fabric,
                spec: LoaderSpec::Latest,
            }
        );
    }

    #[test]
    fn forge_recommended_channel() {
        assert_eq!(
            parse(r#"loader = { forge = "recommended" }"#),
            Loader {
                kind: LoaderKind::Forge,
                spec: LoaderSpec::Recommended,
            }
        );
    }

    #[test]
    fn forge_explicit_latest() {
        assert_eq!(
            parse(r#"loader = { forge = "latest" }"#),
            Loader {
                kind: LoaderKind::Forge,
                spec: LoaderSpec::Latest,
            }
        );
    }

    #[test]
    fn forge_pinned_build() {
        assert_eq!(
            parse(r#"loader = { forge = "47.3.12" }"#),
            Loader {
                kind: LoaderKind::Forge,
                spec: LoaderSpec::Pinned("47.3.12".to_owned()),
            }
        );
    }

    #[test]
    fn fabric_pinned_version() {
        assert_eq!(
            parse(r#"loader = { fabric = "0.15.7" }"#),
            Loader {
                kind: LoaderKind::Fabric,
                spec: LoaderSpec::Pinned("0.15.7".to_owned()),
            }
        );
    }

    #[test]
    fn fabric_recommended_rejected() {
        let err = parse_err(r#"loader = { fabric = "recommended" }"#);
        assert!(err.contains("Forge-only"), "got: {err}");
    }
}
