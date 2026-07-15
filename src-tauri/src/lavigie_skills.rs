//! TASK-35: resolve the vendored per-provider La Vigie skill bundle.
//!
//! Sibling of `lavigie_plugin` (which resolves the Claude *plugin*). These
//! bundles are generated at build time by `rulesync` from `.rulesync/skills/`
//! and shipped as the Tauri resource `resources/lavigie-skills/<provider>/`,
//! each already laid out in that provider's native discovery structure
//! (e.g. `.agents/skills/`, `.opencode/skills/`, `.vibe/skills/`).

use std::path::{Path, PathBuf};
use tauri::path::BaseDirectory;
use tauri::{AppHandle, Manager};

/// A usable bundle dir exists and contains at least one entry.
pub fn is_valid_bundle_dir(dir: &Path) -> bool {
    dir.is_dir()
        && std::fs::read_dir(dir)
            .map(|mut it| it.next().is_some())
            .unwrap_or(false)
}

/// Resolve a vendored bundle dir under `resources/<root>/<provider>`, or `None`.
/// Tries the packaged Resource base dir (two candidate prefixes, as bundlers
/// differ on whether the leading `resources/` segment survives), then a
/// dev-tree fallback next to the crate. Never panics.
fn resolve_bundle_dir(app: &AppHandle, root: &str, provider: &str) -> Option<PathBuf> {
    let rels = [
        format!("resources/{root}/{provider}"),
        format!("{root}/{provider}"),
    ];
    for rel in &rels {
        if let Ok(p) = app.path().resolve(rel, BaseDirectory::Resource) {
            if is_valid_bundle_dir(&p) {
                return Some(p);
            }
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join(root)
        .join(provider);
    if is_valid_bundle_dir(&dev) {
        return Some(dev);
    }
    None
}

/// Resolve the vendored per-provider *skill* bundle (TASK-35).
pub fn resolve_skills_bundle_dir(app: &AppHandle, provider: &str) -> Option<PathBuf> {
    resolve_bundle_dir(app, "lavigie-skills", provider)
}

/// Resolve the vendored per-provider *MCP config* bundle (TASK-193).
pub fn resolve_mcp_bundle_dir(app: &AppHandle, provider: &str) -> Option<PathBuf> {
    resolve_bundle_dir(app, "lavigie-mcp", provider)
}

#[cfg(test)]
mod tests {
    use super::is_valid_bundle_dir;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn valid_when_non_empty_dir() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".agents/skills/ship")).unwrap();
        fs::write(tmp.path().join(".agents/skills/ship/SKILL.md"), "s").unwrap();
        assert!(is_valid_bundle_dir(tmp.path()));
    }

    #[test]
    fn invalid_when_missing_or_empty() {
        let tmp = TempDir::new().unwrap();
        assert!(!is_valid_bundle_dir(&tmp.path().join("nope")));
        assert!(!is_valid_bundle_dir(tmp.path())); // exists but empty
    }
}
