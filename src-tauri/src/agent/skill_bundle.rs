//! TASK-35: materialize a vendored per-provider skill bundle into a worktree.
//!
//! Engines other than Claude discover project-local skills from directories in
//! the working tree (`.agents/skills/`, `.opencode/skills/`, `.vibe/skills/`).
//! We copy the vendored bundle for the resolved provider into the worktree and
//! drop a `.gitignore` of `*` into each injected top-level dir so the injected
//! files never appear in the Diff (worktree-vs-base) or `git status`.
//!
//! Best-effort: a missing bundle or copy error is not fatal — the caller logs
//! and launches the agent skill-free.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The `.gitignore` body written into each injected top-level dir. `*` ignores
/// everything under the dir (including this file), so a fully-ignored directory
/// is omitted from `git status` entirely.
pub const BUNDLE_IGNORE: &str = "*\n";

/// Relative paths (to `worktree`) of every git-tracked file in `worktree`.
///
/// Best-effort: if `worktree` is not a git repo, `git` is missing, or the call
/// otherwise fails, this returns an empty set — so a non-git worktree behaves
/// exactly as before (nothing is treated as tracked, everything is copied).
/// TASK-201: used to never overwrite a file a repo genuinely commits under one of
/// the injected dotdirs (`.agents/`, `.opencode/`, `.vibe/`).
pub(crate) fn tracked_paths(worktree: &Path) -> HashSet<PathBuf> {
    // `-z` → NUL-separated, so any filename (spaces, newlines) round-trips.
    let output = match Command::new("git")
        .arg("-C")
        .arg(worktree)
        .args(["ls-files", "-z"])
        .output()
    {
        Ok(o) if o.status.success() => o.stdout,
        _ => return HashSet::new(),
    };
    output
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .filter_map(|s| std::str::from_utf8(s).ok())
        .map(PathBuf::from)
        .collect()
}

/// Recursively copy `src` into `dst`, creating directories as needed. `rel` is
/// the path of `dst` relative to the worktree root; any file whose relative path
/// is in `tracked` is skipped rather than overwritten (TASK-201), so a repo that
/// commits one of the injected dotdirs is never mutated.
fn copy_tree(
    src: &Path,
    dst: &Path,
    rel: &Path,
    tracked: &HashSet<PathBuf>,
) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let name = entry.file_name();
        let to = dst.join(&name);
        let child_rel = rel.join(&name);
        if entry.file_type()?.is_dir() {
            copy_tree(&from, &to, &child_rel, tracked)?;
        } else if !tracked.contains(&child_rel) {
            fs::copy(&from, &to)?;
        }
        // else: destination collides with a tracked file → skip, never overwrite.
    }
    Ok(())
}

/// Copy each top-level **directory** of `bundle_root` into `worktree`, then write
/// `<worktree>/<dir>/.gitignore = "*\n"` for each. Returns the injected dir names
/// (sorted). `bundle_root` absent, or with no directory entries → `Ok(vec![])`.
pub fn materialize(bundle_root: &Path, worktree: &Path) -> io::Result<Vec<String>> {
    let read = match fs::read_dir(bundle_root) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    // TASK-201: never mutate a file the repo genuinely commits under an injected
    // dotdir. Collect the tracked set once, up front (best-effort: empty for a
    // non-git worktree → prior copy-everything behavior).
    let tracked = tracked_paths(worktree);
    let mut injected: Vec<String> = Vec::new();
    for entry in read {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue; // rulesync only emits dotdirs; skip stray files defensively.
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let dst = worktree.join(&name);
        fs::create_dir_all(&dst)?;
        // Write the git-exclude BEFORE copying, so even a partially-completed
        // copy (if copy_tree errors midway) stays out of the worktree's
        // Diff/status. Don't clobber a .gitignore the repo may legitimately
        // commit for this dotdir — a tracked one exists on disk, so the
        // exists() check preserves it (TASK-201).
        let gitignore = dst.join(".gitignore");
        if !gitignore.exists() {
            fs::write(&gitignore, BUNDLE_IGNORE)?;
        }
        copy_tree(&entry.path(), &dst, Path::new(&name), &tracked)?;
        injected.push(name);
    }
    injected.sort();
    Ok(injected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn write(p: &std::path::Path, body: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    #[test]
    fn copies_bundle_dirs_and_git_excludes_them() {
        let bundle = TempDir::new().unwrap();
        let wt = TempDir::new().unwrap();
        write(&bundle.path().join(".agents/skills/ship/SKILL.md"), "s");
        write(&bundle.path().join(".agents/skills/rename/SKILL.md"), "r");

        let injected = materialize(bundle.path(), wt.path()).unwrap();
        assert_eq!(injected, vec![".agents".to_string()]);
        assert_eq!(
            fs::read_to_string(wt.path().join(".agents/skills/ship/SKILL.md")).unwrap(),
            "s"
        );
        assert_eq!(fs::read_to_string(wt.path().join(".agents/.gitignore")).unwrap(), "*\n");
    }

    #[test]
    fn missing_bundle_is_a_noop_not_an_error() {
        let wt = TempDir::new().unwrap();
        let injected = materialize(std::path::Path::new("/no/such/bundle"), wt.path()).unwrap();
        assert!(injected.is_empty());
    }

    #[test]
    fn preexisting_gitignore_is_not_clobbered() {
        let bundle = TempDir::new().unwrap();
        let wt = TempDir::new().unwrap();
        write(&bundle.path().join(".agents/skills/ship/SKILL.md"), "s");
        write(&wt.path().join(".agents/.gitignore"), "node_modules\n");

        let injected = materialize(bundle.path(), wt.path()).unwrap();
        assert_eq!(injected, vec![".agents".to_string()]);
        assert_eq!(
            fs::read_to_string(wt.path().join(".agents/skills/ship/SKILL.md")).unwrap(),
            "s"
        );
        assert_eq!(
            fs::read_to_string(wt.path().join(".agents/.gitignore")).unwrap(),
            "node_modules\n"
        );
    }

    #[test]
    fn ignores_top_level_files_only_copies_dirs() {
        let bundle = TempDir::new().unwrap();
        let wt = TempDir::new().unwrap();
        write(&bundle.path().join(".vibe/skills/spec-init/SKILL.md"), "x");
        fs::write(bundle.path().join("README.txt"), "ignore me").unwrap();
        let injected = materialize(bundle.path(), wt.path()).unwrap();
        assert_eq!(injected, vec![".vibe".to_string()]);
        assert!(!wt.path().join("README.txt").exists());
    }

    /// Run `git` in `dir`, asserting success. Used to build a fixture worktree.
    fn git(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        out
    }

    // TASK-201: a repo that genuinely COMMITS one of the injected dotdirs must not
    // have any tracked file modified/overwritten, and the injected bundle content
    // must stay out of `git status` (i.e. out of La Vigie's Diff tab).
    #[test]
    fn does_not_overwrite_tracked_files_in_committed_dotdir() {
        let wt = TempDir::new().unwrap();
        // Fixture worktree that commits `.agents/skills/ship/SKILL.md`.
        git(wt.path(), &["init", "-b", "main"]);
        git(wt.path(), &["config", "user.email", "test@example.com"]);
        git(wt.path(), &["config", "user.name", "Test"]);
        write(&wt.path().join(".agents/skills/ship/SKILL.md"), "USER CONTENT");
        git(wt.path(), &["add", "-A"]);
        git(wt.path(), &["commit", "-m", "commit .agents"]);

        // Bundle collides with the tracked ship skill and adds a new one.
        let bundle = TempDir::new().unwrap();
        write(&bundle.path().join(".agents/skills/ship/SKILL.md"), "BUNDLE");
        write(&bundle.path().join(".agents/skills/rename/SKILL.md"), "BUNDLE-RENAME");

        let injected = materialize(bundle.path(), wt.path()).unwrap();
        assert_eq!(injected, vec![".agents".to_string()]);

        // The tracked file is preserved verbatim — never overwritten by the bundle.
        assert_eq!(
            fs::read_to_string(wt.path().join(".agents/skills/ship/SKILL.md")).unwrap(),
            "USER CONTENT"
        );
        // The non-colliding bundle file is still injected.
        assert_eq!(
            fs::read_to_string(wt.path().join(".agents/skills/rename/SKILL.md")).unwrap(),
            "BUNDLE-RENAME"
        );
        // And the worktree is clean: no tracked file modified, and the injected
        // content is git-ignored (`*`), so nothing shows in status / the Diff.
        let status = git(wt.path(), &["status", "--porcelain"]);
        assert!(
            status.stdout.is_empty(),
            "worktree not clean after injection: {}",
            String::from_utf8_lossy(&status.stdout)
        );
    }

    // TASK-201: a tracked `.gitignore` inside a committed dotdir is preserved even
    // in a real git worktree (the exists() guard sees the checked-out file).
    #[test]
    fn preserves_tracked_gitignore_in_committed_dotdir() {
        let wt = TempDir::new().unwrap();
        git(wt.path(), &["init", "-b", "main"]);
        git(wt.path(), &["config", "user.email", "test@example.com"]);
        git(wt.path(), &["config", "user.name", "Test"]);
        // Repo commits `.agents/` with its own `.gitignore` (ignores nothing).
        write(&wt.path().join(".agents/.gitignore"), "# repo-owned\n");
        write(&wt.path().join(".agents/keep.txt"), "keep");
        git(wt.path(), &["add", "-A"]);
        git(wt.path(), &["commit", "-m", "commit .agents"]);

        let bundle = TempDir::new().unwrap();
        write(&bundle.path().join(".agents/skills/ship/SKILL.md"), "BUNDLE");

        materialize(bundle.path(), wt.path()).unwrap();

        // The repo's tracked `.gitignore` and file are untouched.
        assert_eq!(
            fs::read_to_string(wt.path().join(".agents/.gitignore")).unwrap(),
            "# repo-owned\n"
        );
        assert_eq!(
            fs::read_to_string(wt.path().join(".agents/keep.txt")).unwrap(),
            "keep"
        );
    }
}
