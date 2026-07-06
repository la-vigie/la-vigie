//! Custom user-provided notification sounds: copy-into-app-data storage, a
//! JSON registry in `app_settings`, and validation. Pure functions over a
//! `TaskStore` + the managed sounds directory so they unit-test with a temp dir.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::store::TaskStore;

/// Max size of an importable sound file.
pub const MAX_SOUND_BYTES: u64 = 5 * 1024 * 1024;

/// Extensions we let the picker accept and store. Kept in sync with the
/// frontend picker filter.
pub const ALLOWED_EXTS: &[&str] = &["mp3", "wav", "ogg", "m4a", "aac", "flac"];

/// app_settings key holding the JSON `Vec<CustomSound>` registry.
const REGISTRY_KEY: &str = "custom_sounds";

/// id prefix that namespaces custom sounds away from bundled palette ids.
const ID_PREFIX: &str = "custom:";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CustomSound {
    /// `custom:<uuid>`
    pub id: String,
    pub label: String,
    /// lowercased extension, no dot (e.g. "mp3")
    pub ext: String,
}

fn load_registry(store: &TaskStore) -> Result<Vec<CustomSound>> {
    match store.get_app_setting(REGISTRY_KEY)? {
        Some(json) => serde_json::from_str(&json).context("deserializing custom_sounds registry"),
        None => Ok(Vec::new()),
    }
}

fn save_registry(store: &TaskStore, list: &[CustomSound]) -> Result<()> {
    let json = serde_json::to_string(list).context("serializing custom_sounds")?;
    store.set_app_setting(REGISTRY_KEY, &json)
}

/// uuid part after the `custom:` prefix → the on-disk file name `<uuid>.<ext>`.
fn file_for(sounds_dir: &Path, entry: &CustomSound) -> PathBuf {
    let uuid = entry.id.strip_prefix(ID_PREFIX).unwrap_or(&entry.id);
    sounds_dir.join(format!("{uuid}.{}", entry.ext))
}

pub fn list_sounds(store: &TaskStore) -> Result<Vec<CustomSound>> {
    load_registry(store)
}

pub fn import_sound(
    store: &TaskStore,
    sounds_dir: &Path,
    src: &Path,
    label: &str,
) -> Result<CustomSound> {
    let ext = src
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !ALLOWED_EXTS.contains(&ext.as_str()) {
        bail!("unsupported audio format: .{ext}");
    }

    let meta = std::fs::metadata(src).with_context(|| format!("reading {}", src.display()))?;
    if meta.len() > MAX_SOUND_BYTES {
        bail!("file too large (max 5 MB)");
    }

    std::fs::create_dir_all(sounds_dir).context("creating sounds dir")?;
    let uuid = uuid::Uuid::new_v4().to_string();
    let entry = CustomSound {
        id: format!("{ID_PREFIX}{uuid}"),
        label: if label.trim().is_empty() {
            src.file_stem().and_then(|s| s.to_str()).unwrap_or("sound").to_string()
        } else {
            label.trim().to_string()
        },
        ext,
    };
    std::fs::copy(src, file_for(sounds_dir, &entry))
        .with_context(|| format!("copying {}", src.display()))?;

    let mut list = load_registry(store)?;
    list.push(entry.clone());
    save_registry(store, &list)?;
    Ok(entry)
}

pub fn read_bytes(store: &TaskStore, sounds_dir: &Path, id: &str) -> Result<Vec<u8>> {
    let list = load_registry(store)?;
    let entry = list
        .iter()
        .find(|e| e.id == id)
        .ok_or_else(|| anyhow::anyhow!("unknown sound id: {id}"))?;
    std::fs::read(file_for(sounds_dir, entry)).context("reading sound file")
}

pub fn delete_sound(store: &TaskStore, sounds_dir: &Path, id: &str) -> Result<()> {
    let mut list = load_registry(store)?;
    let idx = list
        .iter()
        .position(|e| e.id == id)
        .ok_or_else(|| anyhow::anyhow!("unknown sound id: {id}"))?;
    let entry = list.remove(idx);
    // Best-effort file delete; a missing file shouldn't block deregistration.
    let _ = std::fs::remove_file(file_for(sounds_dir, &entry));
    save_registry(store, &list)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store_in(dir: &TempDir) -> crate::store::TaskStore {
        crate::store::TaskStore::open(&dir.path().join("t.db")).unwrap()
    }

    fn write_src(dir: &TempDir, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn import_rejects_unknown_extension() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        let src = write_src(&dir, "note.txt", b"nope");
        let err = import_sound(&store, dir.path(), &src, "note").unwrap_err();
        assert!(err.to_string().contains("unsupported"), "{err}");
    }

    #[test]
    fn import_rejects_oversize_file() {
        let dir = TempDir::new().unwrap();
        let store = store_in(&dir);
        let big = vec![0u8; (MAX_SOUND_BYTES + 1) as usize];
        let src = write_src(&dir, "big.mp3", &big);
        let err = import_sound(&store, dir.path(), &src, "big").unwrap_err();
        assert!(err.to_string().contains("too large"), "{err}");
    }

    #[test]
    fn import_then_list_read_delete_round_trips() {
        let dir = TempDir::new().unwrap();
        let sounds = dir.path().join("sounds");
        std::fs::create_dir_all(&sounds).unwrap();
        let store = store_in(&dir);
        let src = write_src(&dir, "ding.MP3", b"audio-bytes");

        let entry = import_sound(&store, &sounds, &src, "ding").unwrap();
        assert!(entry.id.starts_with("custom:"));
        assert_eq!(entry.ext, "mp3"); // lowercased
        assert_eq!(entry.label, "ding");

        let listed = list_sounds(&store).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, entry.id);

        let bytes = read_bytes(&store, &sounds, &entry.id).unwrap();
        assert_eq!(bytes, b"audio-bytes");

        delete_sound(&store, &sounds, &entry.id).unwrap();
        assert!(list_sounds(&store).unwrap().is_empty());
        // File is gone too.
        assert!(read_bytes(&store, &sounds, &entry.id).is_err());
    }

    #[test]
    fn read_and_delete_reject_unknown_id() {
        let dir = TempDir::new().unwrap();
        let sounds = dir.path().join("sounds");
        std::fs::create_dir_all(&sounds).unwrap();
        let store = store_in(&dir);
        assert!(read_bytes(&store, &sounds, "custom:does-not-exist").is_err());
        // Deleting an unknown id is an error (nothing to remove).
        assert!(delete_sound(&store, &sounds, "custom:does-not-exist").is_err());
    }
}
