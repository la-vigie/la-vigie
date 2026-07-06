//! Resolve the absolute path to the `claude` binary, robust to a GUI-launched
//! app's minimal PATH (`/usr/bin:/bin:/usr/sbin:/sbin`).
//!
//! The process `PATH` is repaired at startup by `shell_env::hydrate`, so by the
//! time this runs the current `PATH` already reflects the user's login shell.
//! This module just searches that `PATH` plus a few absolute fallbacks.
//!
//! The pure resolver [`resolve_in`] is unit-tested (see the `tests` module
//! below). The thin [`find_binary`] wrapper that reads the real environment
//! (current PATH, home-dir candidates) is **not** unit-tested because it
//! touches live system state; it is covered by manual integration testing in
//! `tauri dev` and the built `.app`.

use std::path::{Path, PathBuf};

// ── pure, testable resolver ───────────────────────────────────────────────────

/// Search `path_dirs` (PATH entries) for an executable file named `name`,
/// then fall back to `candidates` (absolute paths to try in order).
/// Returns the first existing regular file that is executable (unix: mode & 0o111).
pub fn resolve_in(name: &str, path_dirs: &[PathBuf], candidates: &[PathBuf]) -> Option<PathBuf> {
    for dir in path_dirs {
        let p = dir.join(name);
        if is_executable_file(&p) {
            return Some(p);
        }
    }
    for candidate in candidates {
        if is_executable_file(candidate) {
            return Some(candidate.clone());
        }
    }
    None
}

/// Returns true if `path` is an existing regular file with at least one
/// executable bit set (unix: mode & 0o111 != 0).
fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() => meta.permissions().mode() & 0o111 != 0,
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        // On non-unix platforms just check existence.
        path.is_file()
    }
}

// ── thin wrapper that reads the real environment ──────────────────────────────

/// Split a colon-separated PATH string into a `Vec<PathBuf>`.
fn split_path(path_str: &str) -> Vec<PathBuf> {
    path_str
        .split(':')
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Resolve the absolute path to an agent binary named `name`, robust to a
/// GUI-launched app's minimal PATH. Searches the current process `PATH` (repaired
/// at startup by `shell_env::hydrate`), then generic install locations, then
/// claude's historical local-install path when `name == "claude"`. Falls back to
/// the bare `name` if nothing is found.
pub fn find_binary(name: &str) -> PathBuf {
    let current_path_dirs = std::env::var("PATH")
        .map(|p| split_path(&p))
        .unwrap_or_default();

    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"));

    let mut candidates = vec![
        home.join(format!(".local/bin/{name}")),
        PathBuf::from(format!("/opt/homebrew/bin/{name}")),
        PathBuf::from(format!("/usr/local/bin/{name}")),
    ];
    if name == "claude" {
        candidates.push(home.join(".claude/local/claude"));
    }
    // Mistral Vibe binary is typically installed via pip or homebrew as 'vibe'.
    // The generic paths above cover standard locations. Add Mistral-specific
    // fallback for future-proofing (e.g., potential ~/.vibe/bin installation).
    if name == "vibe" {
        candidates.push(home.join(".vibe/bin/vibe"));
    }

    resolve_in(name, &current_path_dirs, &candidates).unwrap_or_else(|| PathBuf::from(name))
}

// ── unit tests (pure resolver only) ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// Create an executable file at `dir/name` (mode 0o755 on unix).
    fn make_executable(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"#!/bin/sh\n").expect("write failed");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("chmod failed");
        path
    }

    /// Create a non-executable file at `dir/name` (mode 0o644 on unix).
    fn make_non_executable(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, b"#!/bin/sh\n").expect("write failed");
        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644))
            .expect("chmod failed");
        path
    }

    // 1. resolve_in finds `claude` in a PATH dir.
    #[test]
    fn finds_claude_in_path_dir() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let expected = make_executable(tmp.path(), "claude");

        let result = resolve_in("claude", &[tmp.path().to_path_buf()], &[]);
        assert_eq!(result, Some(expected));
    }

    // 2. A non-executable `claude` file in a PATH dir is skipped (unix).
    #[test]
    #[cfg(unix)]
    fn skips_non_executable_in_path_dir() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        make_non_executable(tmp.path(), "claude");

        let result = resolve_in("claude", &[tmp.path().to_path_buf()], &[]);
        assert_eq!(result, None);
    }

    // 3. Not in PATH but present as a candidate → returns the candidate.
    #[test]
    fn finds_claude_as_candidate_when_not_in_path() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let candidate = make_executable(tmp.path(), "claude");

        // Pass an empty PATH dir list so only candidates are checked.
        let result = resolve_in("claude", &[], &[candidate.clone()]);
        assert_eq!(result, Some(candidate));
    }

    // 4. Neither PATH dir nor candidate → returns None.
    #[test]
    fn returns_none_when_nothing_found() {
        let result = resolve_in("claude", &[], &[]);
        assert_eq!(result, None);
    }

    // 5. PATH order precedence: earlier dir wins.
    #[test]
    fn path_order_precedence_earlier_dir_wins() {
        let first = tempfile::TempDir::new().expect("tempdir");
        let second = tempfile::TempDir::new().expect("tempdir");

        let first_exe = make_executable(first.path(), "claude");
        let _second_exe = make_executable(second.path(), "claude");

        let result = resolve_in(
            "claude",
            &[first.path().to_path_buf(), second.path().to_path_buf()],
            &[],
        );
        assert_eq!(result, Some(first_exe));
    }

    // 6. Binary name is honored: different names in the same dir are distinct.
    #[test]
    fn resolves_an_arbitrary_binary_name() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let expected = make_executable(tmp.path(), "aider");
        let result = resolve_in("aider", &[tmp.path().to_path_buf()], &[]);
        assert_eq!(result, Some(expected));
        // A different name in the same dir is not found.
        assert_eq!(resolve_in("codex", &[tmp.path().to_path_buf()], &[]), None);
    }
}
