//! Resolve the absolute path to the bundled La Vigie skill *plugin* (TASK-153).
//!
//! The plugin ships as a Tauri resource (`resources/lavigie-plugin/`) containing
//! `.claude-plugin/plugin.json` + `skills/`. La Vigie passes `--plugin-dir <this>`
//! to launched `claude` agents when the "inject La Vigie skills" toggle is on, so
//! the repo's way-of-working skills are available regardless of the operator's
//! personal `~/.claude`. Resolution must work both under `tauri dev` and inside a
//! packaged `.app`; if nothing validates, the caller simply omits the flag.

use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};
use tauri::path::BaseDirectory;

/// A directory is a usable plugin iff it contains `.claude-plugin/plugin.json`.
pub fn is_valid_plugin_dir(dir: &Path) -> bool {
    dir.join(".claude-plugin").join("plugin.json").is_file()
}

/// Resolve the bundled plugin dir, or `None` if it can't be found/validated.
/// Tries the packaged Resource base dir (a couple of candidate relative paths,
/// since bundlers differ on whether the leading `resources/` segment is kept),
/// then a dev-tree fallback next to the crate. Never panics.
pub fn resolve_plugin_dir(app: &AppHandle) -> Option<PathBuf> {
    let resource_candidates = ["resources/lavigie-plugin", "lavigie-plugin"];
    for rel in resource_candidates {
        if let Ok(p) = app.path().resolve(rel, BaseDirectory::Resource) {
            if is_valid_plugin_dir(&p) {
                return Some(p);
            }
        }
    }
    // Dev fallback: the source tree location, used by `tauri dev` where the
    // Resource base dir may not contain the bundled copy.
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/lavigie-plugin");
    if is_valid_plugin_dir(&dev) {
        return Some(dev);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::is_valid_plugin_dir;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn valid_when_manifest_present() {
        let tmp = TempDir::new().unwrap();
        let manifest_dir = tmp.path().join(".claude-plugin");
        fs::create_dir_all(&manifest_dir).unwrap();
        fs::write(manifest_dir.join("plugin.json"), "{}").unwrap();
        assert!(is_valid_plugin_dir(tmp.path()));
    }

    #[test]
    fn invalid_when_manifest_missing() {
        let tmp = TempDir::new().unwrap();
        assert!(!is_valid_plugin_dir(tmp.path()));
        // dir exists but no manifest file
        fs::create_dir_all(tmp.path().join(".claude-plugin")).unwrap();
        assert!(!is_valid_plugin_dir(tmp.path()));
    }
}
