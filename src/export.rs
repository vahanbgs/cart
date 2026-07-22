//! Modpack export formats: mrpack (Modrinth), curseforge, prism.
//!
//! Each submodule owns its format's schema, per-mod routing, and archive
//! writing. The CLI dispatch (`src/cli/export.rs`) resolves manifest
//! entries to (URL, cached-jar) pairs and hands the format module a
//! preassembled index + overrides list; the module produces the archive.

pub mod curseforge;
pub mod mrpack;
pub mod prism;
