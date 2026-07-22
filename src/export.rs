//! Modpack export formats: mrpack (Modrinth), curseforge, prism.
//!
//! Each submodule owns its format's schema + pure conversion helpers
//! (`Loader` → `dependencies` block, etc.). Zipping and I/O live in the
//! CLI dispatch (`src/cli/export.rs`) so this module stays call-order
//! agnostic and unit-testable without touching the filesystem.

pub mod mrpack;
