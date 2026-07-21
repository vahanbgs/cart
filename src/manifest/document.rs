use std::path::Path;

use anyhow::Context;
use tokio::fs;
use toml_edit::DocumentMut;

/// Parse `cart.toml` into a `DocumentMut` that preserves comments, blank
/// lines, and key ordering — so future write commands can mutate one entry
/// without reformatting the whole file.
#[allow(dead_code)] // wired up by the upcoming add/remove/disable commands
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
