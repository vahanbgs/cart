use std::path::Path;

use anyhow::{Context, anyhow, bail};
use tokio::fs;
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value, value};

/// Parse `cart.toml` into a `DocumentMut` that preserves comments, blank
/// lines, and key ordering — so future write commands can mutate one entry
/// without reformatting the whole file.
pub async fn load_document(path: &Path) -> anyhow::Result<DocumentMut> {
    let text = fs::read_to_string(path)
        .await
        .with_context(|| format!("failed to read manifest at {}", path.display()))?;
    text.parse::<DocumentMut>()
        .with_context(|| format!("failed to parse manifest at {}", path.display()))
}

pub async fn save_document(path: &Path, document: &DocumentMut) -> anyhow::Result<()> {
    fs::write(path, document.to_string())
        .await
        .with_context(|| format!("failed to write manifest at {}", path.display()))
}

/// Add `[mods].<name> = { modrinth = "<slug>", version = "<v>" }` to the
/// document. Errors if the key already exists — overwrite would silently
/// discard a user-configured entry.
pub fn add_modrinth_mod(
    document: &mut DocumentMut,
    name: &str,
    slug: &str,
    version: &str,
    disabled: bool,
) -> anyhow::Result<()> {
    let mods = mods_table_mut_or_create(document);
    if mods.contains_key(name) {
        bail!("mod already declared in [mods]: {name}");
    }

    let mut inline = InlineTable::new();
    inline.insert("modrinth", Value::from(slug));
    inline.insert("version", Value::from(version));
    if disabled {
        inline.insert("disabled", Value::from(true));
    }
    inline.fmt();
    mods.insert(name, Item::Value(Value::InlineTable(inline)));

    Ok(())
}

/// Add `[mods].<name> = { curseforge = <projectId>, file = <fileId> }` to
/// the document. Same duplicate-key guard as `add_modrinth_mod`. IDs are
/// numeric because CurseForge slugs can rename but IDs are permanent —
/// callers resolve the slug at add-time and pin only the IDs.
pub fn add_curseforge_mod(
    document: &mut DocumentMut,
    name: &str,
    project_id: u32,
    file_id: u32,
    disabled: bool,
) -> anyhow::Result<()> {
    let mods = mods_table_mut_or_create(document);
    if mods.contains_key(name) {
        bail!("mod already declared in [mods]: {name}");
    }

    let mut inline = InlineTable::new();
    inline.insert("curseforge", Value::from(project_id as i64));
    inline.insert("file", Value::from(file_id as i64));
    if disabled {
        inline.insert("disabled", Value::from(true));
    }
    inline.fmt();
    mods.insert(name, Item::Value(Value::InlineTable(inline)));

    Ok(())
}

/// Remove `[mods].<name>` from the document. Errors if `[mods]` is absent
/// or the entry doesn't exist — silently succeeding on a typo would be a
/// footgun.
pub fn remove_mod(document: &mut DocumentMut, name: &str) -> anyhow::Result<()> {
    let mods = mods_table_mut(document)?;
    if mods.remove(name).is_none() {
        bail!("mod not found in [mods]: {name}");
    }
    Ok(())
}

/// Toggle the `disabled` field on `[mods].<name>`. Idempotent: setting to
/// the current state is a no-op. Enabling removes the key entirely rather
/// than writing `disabled = false`, since `false` is the default and
/// omitting it keeps the manifest cleaner.
pub fn set_mod_disabled(
    document: &mut DocumentMut,
    name: &str,
    disabled: bool,
) -> anyhow::Result<()> {
    let mods = mods_table_mut(document)?;
    let entry = mods
        .get_mut(name)
        .ok_or_else(|| anyhow!("mod not found in [mods]: {name}"))?;

    // A mod entry can be either an inline table (`jei = { url = "..." }`)
    // or a subtable (`[mods.jei]\nurl = "..."`); both are valid TOML and
    // deserialize into the same `ModDependency`.
    match entry {
        Item::Value(Value::InlineTable(inline)) => {
            if disabled {
                inline.insert("disabled", Value::from(true));
                // Normalise separator whitespace — otherwise the previously
                // last value's trailing space bleeds in front of the
                // inserted comma (`"url" , disabled = true`).
                inline.fmt();
            } else {
                inline.remove("disabled");
            }
        }
        Item::Table(table) => {
            if disabled {
                table.insert("disabled", value(true));
            } else {
                table.remove("disabled");
            }
        }
        _ => bail!("[mods].{name} is not a table"),
    }
    Ok(())
}

/// Overwrite `[mods].<name>.version` with `version`. Errors if the entry
/// isn't a Modrinth-shaped table — URL entries have no meaningful version
/// to set, and `cart update` should never call this for them.
pub fn set_mod_version(
    document: &mut DocumentMut,
    name: &str,
    version: &str,
) -> anyhow::Result<()> {
    let mods = mods_table_mut(document)?;
    let entry = mods
        .get_mut(name)
        .ok_or_else(|| anyhow!("mod not found in [mods]: {name}"))?;

    match entry {
        Item::Value(Value::InlineTable(inline)) => {
            if !inline.contains_key("modrinth") {
                bail!(
                    "[mods].{name} has no `modrinth` key — only Modrinth entries can be version-pinned"
                );
            }
            inline.insert("version", Value::from(version));
            inline.fmt();
        }
        Item::Table(table) => {
            if !table.contains_key("modrinth") {
                bail!("[mods].{name} has no `modrinth` key");
            }
            table.insert("version", value(version));
        }
        _ => bail!("[mods].{name} is not a table"),
    }
    Ok(())
}

/// Overwrite `[mods].<name>.file` with `file_id`. CurseForge analogue of
/// `set_mod_version`: errors if the entry isn't CurseForge-shaped, since
/// writing a `file` key onto a Modrinth or URL entry would produce a
/// hybrid that fails `ModDependency` deserialization.
pub fn set_mod_file(
    document: &mut DocumentMut,
    name: &str,
    file_id: u32,
) -> anyhow::Result<()> {
    let mods = mods_table_mut(document)?;
    let entry = mods
        .get_mut(name)
        .ok_or_else(|| anyhow!("mod not found in [mods]: {name}"))?;

    match entry {
        Item::Value(Value::InlineTable(inline)) => {
            if !inline.contains_key("curseforge") {
                bail!(
                    "[mods].{name} has no `curseforge` key — only CurseForge entries can have their file id updated"
                );
            }
            inline.insert("file", Value::from(file_id as i64));
            inline.fmt();
        }
        Item::Table(table) => {
            if !table.contains_key("curseforge") {
                bail!("[mods].{name} has no `curseforge` key");
            }
            table.insert("file", value(file_id as i64));
        }
        _ => bail!("[mods].{name} is not a table"),
    }
    Ok(())
}

fn mods_table_mut(document: &mut DocumentMut) -> anyhow::Result<&mut Table> {
    document
        .get_mut("mods")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| anyhow!("[mods] table missing from cart.toml"))
}

fn mods_table_mut_or_create(document: &mut DocumentMut) -> &mut Table {
    if !document.contains_key("mods") {
        let mut table = Table::new();
        table.set_implicit(false);
        document.insert("mods", Item::Table(table));
    }
    document["mods"]
        .as_table_mut()
        .expect("[mods] just inserted as a table")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> DocumentMut {
        input.parse().unwrap()
    }

    /// Load → save must be byte-identical for any well-formed cart.toml:
    /// comments, blank lines, key order, and inline-table spacing all
    /// preserved. This is the load-bearing guarantee of the toml_edit
    /// migration; if it breaks, `cart add`/`remove`/`disable` will silently
    /// reformat the user's file.
    #[tokio::test]
    async fn roundtrip_preserves_comments_and_formatting() {
        let input = "\
# top-level comment
minecraft = \"1.12.2\"  # trailing comment
forge = \"latest\"

# section comment
[mods]
# jei is the standard item viewer
jei = { url = \"https://example.com/jei.jar\" }
appleskin = { url = \"https://example.com/appleskin.jar\", disabled = true }
";
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cart.toml");
        tokio::fs::write(&path, input).await.unwrap();

        let doc = load_document(&path).await.unwrap();
        let out_path = dir.path().join("out.toml");
        save_document(&out_path, &doc).await.unwrap();

        let output = tokio::fs::read_to_string(&out_path).await.unwrap();
        assert_eq!(input, output);
    }

    // ---------- add_modrinth_mod ----------

    #[test]
    fn add_creates_mods_table_when_missing() {
        let mut doc = parse("minecraft = \"1.20.1\"\n");
        add_modrinth_mod(&mut doc, "jei", "jei", "15.2.0.27", false).unwrap();
        let out = doc.to_string();
        assert!(out.contains("[mods]"), "missing [mods] header:\n{out}");
        assert!(
            out.contains(r#"jei = { modrinth = "jei", version = "15.2.0.27" }"#),
            "wrong inline shape:\n{out}"
        );
    }

    #[test]
    fn add_appends_next_to_existing_entries() {
        let mut doc = parse(
            "[mods]\nmantle = { modrinth = \"mantle\", version = \"1.0\" }\n",
        );
        add_modrinth_mod(&mut doc, "jei", "jei", "15.2.0.27", false).unwrap();
        let out = doc.to_string();
        assert!(out.contains(r#"mantle = { modrinth = "mantle", version = "1.0" }"#));
        assert!(out.contains(r#"jei = { modrinth = "jei", version = "15.2.0.27" }"#));
    }

    /// The "already declared" guard is what prevents `cart add jei` from
    /// silently clobbering an entry the user already has — the doc
    /// comment on `add_modrinth_mod` calls this out explicitly.
    #[test]
    fn add_rejects_duplicate_key() {
        let mut doc = parse("[mods]\njei = { modrinth = \"jei\", version = \"1.0\" }\n");
        let err = add_modrinth_mod(&mut doc, "jei", "jei", "2.0", false).unwrap_err();
        assert!(
            err.to_string().contains("already declared"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn add_writes_disabled_only_when_requested() {
        let mut doc = parse("[mods]\n");
        add_modrinth_mod(&mut doc, "wip", "some-mod", "1.0", true).unwrap();
        add_modrinth_mod(&mut doc, "jei", "jei", "1.0", false).unwrap();
        let out = doc.to_string();
        assert!(
            out.contains(r#"wip = { modrinth = "some-mod", version = "1.0", disabled = true }"#),
            "expected disabled=true on wip:\n{out}"
        );
        assert!(
            out.contains(r#"jei = { modrinth = "jei", version = "1.0" }"#),
            "jei should have no `disabled` key:\n{out}"
        );
    }

    // ---------- add_curseforge_mod ----------

    #[test]
    fn add_curseforge_creates_mods_table_when_missing() {
        let mut doc = parse("minecraft = \"1.20.1\"\n");
        add_curseforge_mod(&mut doc, "jei", 238222, 8419086, false).unwrap();
        let out = doc.to_string();
        assert!(out.contains("[mods]"), "missing [mods] header:\n{out}");
        assert!(
            out.contains("jei = { curseforge = 238222, file = 8419086 }"),
            "wrong inline shape:\n{out}"
        );
    }

    #[test]
    fn add_curseforge_writes_disabled_only_when_requested() {
        let mut doc = parse("[mods]\n");
        add_curseforge_mod(&mut doc, "wip", 238222, 8419086, true).unwrap();
        add_curseforge_mod(&mut doc, "jei", 238222, 8419086, false).unwrap();
        let out = doc.to_string();
        assert!(
            out.contains(
                "wip = { curseforge = 238222, file = 8419086, disabled = true }"
            ),
            "expected disabled=true on wip:\n{out}"
        );
        assert!(
            out.contains("jei = { curseforge = 238222, file = 8419086 }"),
            "jei should have no `disabled` key:\n{out}"
        );
    }

    #[test]
    fn add_curseforge_rejects_duplicate_key() {
        let mut doc = parse(
            "[mods]\njei = { curseforge = 238222, file = 8419086 }\n",
        );
        let err = add_curseforge_mod(&mut doc, "jei", 238222, 9999999, false).unwrap_err();
        assert!(
            err.to_string().contains("already declared"),
            "unexpected error: {err}"
        );
    }

    // ---------- remove_mod ----------

    #[test]
    fn remove_deletes_entry_and_leaves_others() {
        let input = "[mods]\n\
            jei = { modrinth = \"jei\", version = \"1.0\" }\n\
            mantle = { modrinth = \"mantle\", version = \"2.0\" }\n";
        let mut doc = parse(input);
        remove_mod(&mut doc, "jei").unwrap();
        let out = doc.to_string();
        assert!(!out.contains("jei"), "jei should be gone:\n{out}");
        assert!(out.contains(r#"mantle = { modrinth = "mantle", version = "2.0" }"#));
    }

    #[test]
    fn remove_errors_when_key_missing() {
        let mut doc = parse("[mods]\njei = { modrinth = \"jei\", version = \"1.0\" }\n");
        let err = remove_mod(&mut doc, "appleskin").unwrap_err();
        assert!(err.to_string().contains("not found"), "unexpected: {err}");
    }

    /// The two "missing" cases return distinct messages — no [mods] table
    /// at all vs. entry not in [mods] — because the CLI surfaces them
    /// verbatim and the diagnostics matter to the user.
    #[test]
    fn remove_errors_when_mods_table_absent() {
        let mut doc = parse("minecraft = \"1.20.1\"\n");
        let err = remove_mod(&mut doc, "jei").unwrap_err();
        assert!(
            err.to_string().contains("[mods] table missing"),
            "unexpected: {err}"
        );
    }

    // ---------- set_mod_disabled ----------

    #[test]
    fn disable_adds_disabled_true_on_inline_table() {
        let mut doc = parse("[mods]\njei = { modrinth = \"jei\", version = \"1.0\" }\n");
        set_mod_disabled(&mut doc, "jei", true).unwrap();
        assert!(
            doc.to_string().contains("disabled = true"),
            "{}",
            doc.to_string()
        );
    }

    /// The doc comment promises "enabling *removes* the key entirely
    /// rather than writing `disabled = false`". This is the test that
    /// pins that behavior — a refactor that switches to writing
    /// `disabled = false` would leak clutter into every user's cart.toml.
    #[test]
    fn enable_removes_the_disabled_key_rather_than_writing_false() {
        let mut doc = parse(
            "[mods]\njei = { modrinth = \"jei\", version = \"1.0\", disabled = true }\n",
        );
        set_mod_disabled(&mut doc, "jei", false).unwrap();
        let out = doc.to_string();
        assert!(!out.contains("disabled"), "disabled should be gone:\n{out}");
    }

    /// Idempotence in both directions — the CLI's `enable`/`disable` are
    /// naturally re-runnable, and neither should ever land the user with
    /// `disabled = true, disabled = true` or similar duplicated state.
    #[test]
    fn disable_and_enable_are_idempotent() {
        let mut disabled_doc = parse(
            "[mods]\njei = { modrinth = \"jei\", version = \"1.0\", disabled = true }\n",
        );
        set_mod_disabled(&mut disabled_doc, "jei", true).unwrap();
        assert_eq!(disabled_doc.to_string().matches("disabled").count(), 1);

        let enabled_input = "[mods]\njei = { modrinth = \"jei\", version = \"1.0\" }\n";
        let mut enabled_doc = parse(enabled_input);
        set_mod_disabled(&mut enabled_doc, "jei", false).unwrap();
        assert_eq!(enabled_doc.to_string(), enabled_input);
    }

    /// Subtable form (`[mods.jei]\nmodrinth = "jei"`) is also valid TOML
    /// and users may hand-write it. Both mutation paths must handle it.
    #[test]
    fn disable_works_on_subtable_form() {
        let mut doc =
            parse("[mods.jei]\nmodrinth = \"jei\"\nversion = \"1.0\"\n");
        set_mod_disabled(&mut doc, "jei", true).unwrap();
        assert!(doc.to_string().contains("disabled = true"));
    }

    #[test]
    fn set_disabled_errors_when_key_missing() {
        let mut doc = parse("[mods]\n");
        let err = set_mod_disabled(&mut doc, "jei", true).unwrap_err();
        assert!(err.to_string().contains("not found"), "unexpected: {err}");
    }

    // ---------- set_mod_version ----------

    #[test]
    fn set_version_overwrites_on_inline_table() {
        let mut doc = parse("[mods]\njei = { modrinth = \"jei\", version = \"1.0\" }\n");
        set_mod_version(&mut doc, "jei", "2.0").unwrap();
        let out = doc.to_string();
        assert!(out.contains(r#"version = "2.0""#), "{out}");
        assert!(!out.contains(r#""1.0""#), "old version leaked:\n{out}");
    }

    #[test]
    fn set_version_works_on_subtable_form() {
        let mut doc =
            parse("[mods.jei]\nmodrinth = \"jei\"\nversion = \"1.0\"\n");
        set_mod_version(&mut doc, "jei", "2.0").unwrap();
        assert!(doc.to_string().contains(r#"version = "2.0""#));
    }

    /// `cart update` walks every mod and calls this for Modrinth entries.
    /// If it ever gets called with a URL entry (bug in the caller), the
    /// helper must refuse — writing a `version` key onto a URL entry
    /// would produce an entry that fails to deserialize as `ModDependency`
    /// (neither variant would match).
    #[test]
    fn set_version_rejects_url_entries() {
        let mut doc = parse(
            "[mods]\ncustom = { url = \"https://example.com/jei.jar\" }\n",
        );
        let err = set_mod_version(&mut doc, "custom", "2.0").unwrap_err();
        assert!(
            err.to_string().contains("no `modrinth` key"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn set_version_errors_when_key_missing() {
        let mut doc = parse("[mods]\n");
        let err = set_mod_version(&mut doc, "jei", "2.0").unwrap_err();
        assert!(err.to_string().contains("not found"), "unexpected: {err}");
    }

    // ---------- set_mod_file ----------

    #[test]
    fn set_file_overwrites_on_inline_table() {
        let mut doc = parse(
            "[mods]\njei = { curseforge = 238222, file = 8419086 }\n",
        );
        set_mod_file(&mut doc, "jei", 9000000).unwrap();
        let out = doc.to_string();
        assert!(out.contains("file = 9000000"), "{out}");
        assert!(!out.contains("8419086"), "old file id leaked:\n{out}");
    }

    #[test]
    fn set_file_works_on_subtable_form() {
        let mut doc = parse(
            "[mods.jei]\ncurseforge = 238222\nfile = 8419086\n",
        );
        set_mod_file(&mut doc, "jei", 9000000).unwrap();
        assert!(doc.to_string().contains("file = 9000000"));
    }

    /// The Modrinth counterpart guard exists so that `cart update`
    /// never accidentally cross-writes a CF field onto a Modrinth entry
    /// — same rationale here in reverse.
    #[test]
    fn set_file_rejects_modrinth_entries() {
        let mut doc = parse(
            "[mods]\njei = { modrinth = \"jei\", version = \"1.0\" }\n",
        );
        let err = set_mod_file(&mut doc, "jei", 8419086).unwrap_err();
        assert!(
            err.to_string().contains("no `curseforge` key"),
            "unexpected: {err}"
        );
    }

    #[test]
    fn set_file_errors_when_key_missing() {
        let mut doc = parse("[mods]\n");
        let err = set_mod_file(&mut doc, "jei", 8419086).unwrap_err();
        assert!(err.to_string().contains("not found"), "unexpected: {err}");
    }

    // ---------- surrounding-content preservation across mutation ----------

    /// The load→save round-trip test proves comments survive an untouched
    /// document. This one proves they survive a mutation too — the whole
    /// point of the toml_edit migration was that `cart add`/`update`
    /// don't touch what they haven't been asked to touch.
    #[test]
    fn mutations_preserve_surrounding_comments_and_entries() {
        let input = "\
# top-level comment
minecraft = \"1.20.1\"

# section comment
[mods]
# jei is standard
jei = { modrinth = \"jei\", version = \"1.0\" }
mantle = { url = \"https://example.com/mantle.jar\" }
";
        let mut doc = parse(input);
        add_modrinth_mod(&mut doc, "appleskin", "appleskin", "2.5.1", false).unwrap();
        set_mod_version(&mut doc, "jei", "2.0").unwrap();
        set_mod_disabled(&mut doc, "mantle", true).unwrap();

        let out = doc.to_string();
        for expected in [
            "# top-level comment",
            "minecraft = \"1.20.1\"",
            "# section comment",
            "# jei is standard",
            r#"jei = { modrinth = "jei", version = "2.0" }"#,
            r#"appleskin = { modrinth = "appleskin", version = "2.5.1" }"#,
        ] {
            assert!(
                out.contains(expected),
                "expected {expected:?} in:\n{out}"
            );
        }
        assert!(
            out.contains("disabled = true") && out.contains("mantle"),
            "expected mantle to end up disabled:\n{out}"
        );
    }
}
