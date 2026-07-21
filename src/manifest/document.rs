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
}
