//! Mod-loader selector in `cart.toml`.
//!
//! ```toml
//! # loader = "fabric"                     # → Fabric latest
//! # loader = "forge"                      # → Forge latest
//! # loader = "neoforge"                   # → NeoForge latest
//! # loader = { fabric = "0.15.7" }        # pinned Fabric loader
//! # loader = { forge = "recommended" }    # Forge's stable channel
//! # loader = { forge = "47.3.12" }        # pinned Forge build
//! # loader = { neoforge = "20.4.237" }    # pinned NeoForge build
//! ```

use serde::{Deserialize, Deserializer};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoaderKind {
    Fabric,
    Forge,
    NeoForge,
}

impl LoaderKind {
    /// Loader identifier as Modrinth expects in `/v2/search` facets and
    /// `/v2/project/{slug}/version?loaders=[…]` queries. The vanilla
    /// case (no loader in the manifest) is Modrinth's `"vanilla"` and is
    /// handled by callers via `Option::map(...).unwrap_or("vanilla")`.
    pub fn as_modrinth(self) -> &'static str {
        match self {
            Self::Fabric => "fabric",
            Self::Forge => "forge",
            Self::NeoForge => "neoforge",
        }
    }
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

impl Loader {
    pub fn forge(spec: LoaderSpec) -> Self {
        Self {
            kind: LoaderKind::Forge,
            spec,
        }
    }

    pub fn fabric(spec: LoaderSpec) -> Self {
        Self {
            kind: LoaderKind::Fabric,
            spec,
        }
    }

    pub fn neoforge(spec: LoaderSpec) -> Self {
        Self {
            kind: LoaderKind::NeoForge,
            spec,
        }
    }
}

impl<'de> Deserialize<'de> for Loader {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Bare(LoaderKind),
            Forge { forge: String },
            Fabric { fabric: String },
            NeoForge { neoforge: String },
        }

        fn reject_recommended<E: serde::de::Error>(
            spec: LoaderSpec,
            loader_name: &str,
        ) -> Result<LoaderSpec, E> {
            if matches!(spec, LoaderSpec::Recommended) {
                Err(serde::de::Error::custom(format!(
                    "`recommended` is a Forge-only channel; {loader_name} only has `latest` or a pinned version"
                )))
            } else {
                Ok(spec)
            }
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
            Repr::Fabric { fabric } => Loader {
                kind: LoaderKind::Fabric,
                spec: reject_recommended(LoaderSpec::parse(&fabric), "Fabric")?,
            },
            Repr::NeoForge { neoforge } => Loader {
                kind: LoaderKind::NeoForge,
                spec: reject_recommended(LoaderSpec::parse(&neoforge), "NeoForge")?,
            },
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
        toml::from_str::<Wrapper>(toml_body)
            .unwrap_err()
            .to_string()
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

    #[test]
    fn bare_string_neoforge_is_latest() {
        assert_eq!(
            parse(r#"loader = "neoforge""#),
            Loader {
                kind: LoaderKind::NeoForge,
                spec: LoaderSpec::Latest,
            }
        );
    }

    #[test]
    fn neoforge_pinned_version() {
        assert_eq!(
            parse(r#"loader = { neoforge = "20.4.237" }"#),
            Loader {
                kind: LoaderKind::NeoForge,
                spec: LoaderSpec::Pinned("20.4.237".to_owned()),
            }
        );
    }

    #[test]
    fn neoforge_recommended_rejected() {
        let err = parse_err(r#"loader = { neoforge = "recommended" }"#);
        assert!(err.contains("Forge-only"), "got: {err}");
    }
}
