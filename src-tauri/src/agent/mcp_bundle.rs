//! TASK-193: materialize a vendored per-provider MCP config into a worktree.
//!
//! Sibling of `skill_bundle` (which injects skill *content*). Non-Claude engines
//! discover a project-local MCP config from the working tree
//! (`.codex/config.toml`, `.agents/mcp_config.json`, `opencode.jsonc`,
//! `.vibe/config.toml`). We copy the vendored config for the resolved provider
//! into the worktree — substituting the per-spawn loopback port + agent bearer
//! token — and git-exclude it so it never shows in the Diff. Best-effort: any
//! failure logs and the agent launches without the injected server.

use super::skill_bundle::{tracked_paths, BUNDLE_IGNORE};
use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Sentinels authored into `.rulesync/.mcp.json`, substituted at spawn. NOT the
/// shell `${VAR}` form: rulesync's opencode emitter rewrites `${VAR}` into that
/// tool's native `{env:VAR}` interpolation, which would defeat literal
/// substitution. Bare `__…__` sentinels round-trip verbatim through every target.
/// Keep these byte-identical to that source file.
pub const MCP_PORT_PLACEHOLDER: &str = "__LAVIGIE_MCP_PORT__";
pub const MCP_TOKEN_PLACEHOLDER: &str = "__LAVIGIE_MCP_TOKEN__";

/// Recursively copy `src`→`dst`, substituting placeholders in file *contents*,
/// skipping any file whose worktree-relative path is tracked (TASK-201).
/// Substitute the per-spawn port + token into raw config `bytes`. Lossy UTF-8:
/// rulesync only emits UTF-8 JSON/TOML, and a stray byte must not abort injection
/// (skill_bundle copies bytes verbatim; here we must read to substitute).
fn substitute(bytes: &[u8], port: u16, token: &str) -> String {
    String::from_utf8_lossy(bytes)
        .replace(MCP_PORT_PLACEHOLDER, &port.to_string())
        .replace(MCP_TOKEN_PLACEHOLDER, token)
}

/// Is `rel` (worktree-relative) git-ignored in the worktree's CURRENT state?
/// Best-effort: any git failure → `false`.
fn git_check_ignore(worktree: &Path, rel: &Path) -> bool {
    Command::new("git")
        .arg("-C").arg(worktree)
        .args(["check-ignore", "-q"])
        .arg(rel)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// A worktree-relative target we must NOT write: the repo tracks it (TASK-201), or
/// a user-authored file already sits there that La Vigie did not inject. Our own
/// injected files are always git-excluded, so an on-disk file that is NOT ignored
/// in the pre-injection state is the user's — never clobber it. MUST be evaluated
/// BEFORE writing any `.gitignore`/`info/exclude`, or our own exclude would mask a
/// user's file and make us think it is ours.
fn is_user_owned(worktree: &Path, rel: &Path, tracked: &HashSet<PathBuf>) -> bool {
    if tracked.contains(rel) {
        return true;
    }
    worktree.join(rel).exists() && !git_check_ignore(worktree, rel)
}

/// Collect every file under `src` as its worktree-relative path (prefixed by `rel`).
fn collect_files(src: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let child_rel = rel.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            collect_files(&entry.path(), &child_rel, out)?;
        } else {
            out.push(child_rel);
        }
    }
    Ok(())
}

/// Recursively copy `src`→`dst`, substituting placeholders, skipping any file
/// whose worktree-relative path is in `protected`.
fn copy_subst(
    src: &Path,
    dst: &Path,
    rel: &Path,
    protected: &HashSet<PathBuf>,
    port: u16,
    token: &str,
) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let to = dst.join(&name);
        let child_rel = rel.join(&name);
        if entry.file_type()?.is_dir() {
            copy_subst(&entry.path(), &to, &child_rel, protected, port, token)?;
        } else if !protected.contains(&child_rel) {
            fs::write(&to, substitute(&fs::read(entry.path())?, port, token))?;
        }
    }
    Ok(())
}

/// Append `pattern` to the worktree's shared `info/exclude` if not already
/// present (best-effort; per-worktree exclude is NOT honored by git — only the
/// common dir is). Idempotent so repeated spawns don't pile up duplicates.
fn add_common_exclude(worktree: &Path, pattern: &str) -> io::Result<()> {
    let common = Command::new("git")
        .arg("-C").arg(worktree)
        .args(["rev-parse", "--git-common-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    let Some(common) = common else { return Ok(()) };
    // `--git-common-dir` may be relative to the worktree; resolve against it.
    let common_dir = {
        let p = PathBuf::from(&common);
        if p.is_absolute() { p } else { worktree.join(p) }
    };
    let info = common_dir.join("info");
    fs::create_dir_all(&info)?;
    let exclude = info.join("exclude");
    let existing = fs::read_to_string(&exclude).unwrap_or_default();
    if existing.lines().any(|l| l == pattern) {
        return Ok(());
    }
    let mut body = existing;
    if !body.is_empty() && !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(pattern);
    body.push('\n');
    fs::write(&exclude, body)
}

/// Materialize each top-level entry of `bundle_root` into `worktree` with the
/// per-spawn port + token substituted in, git-excluded so it never shows in the
/// Diff, and returns the injected top-level names (sorted). `bundle_root` absent →
/// `Ok(vec![])`.
///
/// Never overwrites a repo-tracked file (TASK-201) nor a user-authored untracked
/// config already on disk — the `protected` set is computed against the
/// pre-injection git state, before any `.gitignore`/exclude is written, so our own
/// exclusion can't mask a user's file. A top-level **dir** gets `<dir>/.gitignore="*"`
/// (worktree-local); a top-level **file** (e.g. `opencode.jsonc`) is root-anchored
/// into the shared `info/exclude`, written BEFORE the token-bearing file so an
/// exclude failure never leaves the credential visible to `git status`/an agent commit.
pub fn materialize_mcp(
    bundle_root: &Path,
    worktree: &Path,
    port: u16,
    token: &str,
) -> io::Result<Vec<String>> {
    let read = match fs::read_dir(bundle_root) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let tracked = tracked_paths(worktree);

    // Snapshot the top-level bundle entries, then (PRE-PASS, before any write)
    // compute which target paths are user-owned and must be left untouched.
    let mut top: Vec<(PathBuf, bool, String)> = Vec::new();
    for entry in read {
        let entry = entry?;
        let is_dir = entry.file_type()?.is_dir();
        let name = entry.file_name().to_string_lossy().into_owned();
        top.push((entry.path(), is_dir, name));
    }
    let mut all_files: Vec<PathBuf> = Vec::new();
    for (src, is_dir, name) in &top {
        if *is_dir {
            collect_files(src, Path::new(name), &mut all_files)?;
        } else {
            all_files.push(PathBuf::from(name));
        }
    }
    let protected: HashSet<PathBuf> = all_files
        .into_iter()
        .filter(|rel| is_user_owned(worktree, rel, &tracked))
        .collect();

    // WRITE PASS.
    let mut injected: Vec<String> = Vec::new();
    for (src, is_dir, name) in &top {
        if *is_dir {
            // Skip the dir entirely if every file in it is protected — so we never
            // even drop a `.gitignore` into a user's own untracked dotdir.
            let mut files = Vec::new();
            collect_files(src, Path::new(name), &mut files)?;
            if files.iter().all(|rel| protected.contains(rel)) {
                continue;
            }
            let dst = worktree.join(name);
            fs::create_dir_all(&dst)?;
            // Exclude BEFORE copying (matches skill_bundle); don't clobber a
            // repo-committed `.gitignore` for this dotdir (TASK-201).
            let gitignore = dst.join(".gitignore");
            if !gitignore.exists() {
                fs::write(&gitignore, BUNDLE_IGNORE)?;
            }
            copy_subst(src, &dst, Path::new(name), &protected, port, token)?;
        } else {
            if protected.contains(&PathBuf::from(name.as_str())) {
                continue; // repo-tracked or a user-authored config — never clobber.
            }
            // Root-anchored so it hides only this exact top-level path, and written
            // BEFORE the token-bearing file so a git-exclude failure never exposes it.
            add_common_exclude(worktree, &format!("/{name}"))?;
            fs::write(worktree.join(name), substitute(&fs::read(src)?, port, token))?;
        }
        injected.push(name.clone());
    }
    injected.sort();
    Ok(injected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use tempfile::TempDir;

    fn write(p: &std::path::Path, body: &str) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }
    fn git(dir: &std::path::Path, args: &[&str]) {
        let out = Command::new("git").arg("-C").arg(dir).args(args).output().expect("git");
        assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    }
    fn init_repo(dir: &std::path::Path) {
        git(dir, &["init", "-b", "main"]);
        git(dir, &["config", "user.email", "t@e.com"]);
        git(dir, &["config", "user.name", "T"]);
    }

    #[test]
    fn substitutes_port_and_token_and_excludes_dotdir() {
        let bundle = TempDir::new().unwrap();
        let wt = TempDir::new().unwrap();
        init_repo(wt.path());
        write(&bundle.path().join(".codex/config.toml"),
            "url = \"http://127.0.0.1:__LAVIGIE_MCP_PORT__/mcp\"\nAuthorization = \"Bearer __LAVIGIE_MCP_TOKEN__\"\n");

        let injected = materialize_mcp(bundle.path(), wt.path(), 54321, "tok-abc").unwrap();
        assert_eq!(injected, vec![".codex".to_string()]);
        let got = fs::read_to_string(wt.path().join(".codex/config.toml")).unwrap();
        assert!(got.contains("http://127.0.0.1:54321/mcp"), "port substituted: {got}");
        assert!(got.contains("Bearer tok-abc"), "token substituted: {got}");
        assert!(!got.contains("__LAVIGIE_MCP"), "no leftover placeholder: {got}");
        assert_eq!(fs::read_to_string(wt.path().join(".codex/.gitignore")).unwrap(), "*\n");
        // Worktree stays clean: injected files are excluded.
        let st = Command::new("git").arg("-C").arg(wt.path()).args(["status","--porcelain"]).output().unwrap();
        assert!(st.stdout.is_empty(), "dirty: {}", String::from_utf8_lossy(&st.stdout));
    }

    #[test]
    fn top_level_file_is_excluded_via_common_info_exclude() {
        let bundle = TempDir::new().unwrap();
        let wt = TempDir::new().unwrap();
        init_repo(wt.path());
        write(&bundle.path().join("opencode.jsonc"),
            "{ \"url\": \"http://127.0.0.1:__LAVIGIE_MCP_PORT__/mcp\", \"token\": \"__LAVIGIE_MCP_TOKEN__\" }");

        let injected = materialize_mcp(bundle.path(), wt.path(), 9, "z").unwrap();
        assert_eq!(injected, vec!["opencode.jsonc".to_string()]);
        assert!(wt.path().join("opencode.jsonc").exists());
        let st = Command::new("git").arg("-C").arg(wt.path()).args(["status","--porcelain"]).output().unwrap();
        assert!(st.stdout.is_empty(), "opencode.jsonc not excluded: {}", String::from_utf8_lossy(&st.stdout));
    }

    #[test]
    fn missing_bundle_is_a_noop() {
        let wt = TempDir::new().unwrap();
        assert!(materialize_mcp(std::path::Path::new("/no/such"), wt.path(), 1, "t").unwrap().is_empty());
    }

    #[test]
    fn does_not_overwrite_tracked_config() {
        let wt = TempDir::new().unwrap();
        init_repo(wt.path());
        write(&wt.path().join("opencode.jsonc"), "USER CONFIG");
        git(wt.path(), &["add", "-A"]);
        git(wt.path(), &["commit", "-m", "user opencode.jsonc"]);

        let bundle = TempDir::new().unwrap();
        write(&bundle.path().join("opencode.jsonc"), "BUNDLE __LAVIGIE_MCP_TOKEN__");
        materialize_mcp(bundle.path(), wt.path(), 1, "t").unwrap();

        assert_eq!(fs::read_to_string(wt.path().join("opencode.jsonc")).unwrap(), "USER CONFIG");
        let st = Command::new("git").arg("-C").arg(wt.path()).args(["status","--porcelain"]).output().unwrap();
        assert!(st.stdout.is_empty(), "tracked file mutated: {}", String::from_utf8_lossy(&st.stdout));
    }

    // A user-authored *untracked* config (the common case for opencode.jsonc /
    // .codex/config.toml) must never be clobbered — the tracked-file guard alone
    // wouldn't catch it.
    #[test]
    fn does_not_overwrite_untracked_user_file() {
        let wt = TempDir::new().unwrap();
        init_repo(wt.path());
        write(&wt.path().join("opencode.jsonc"), "USER UNTRACKED");

        let bundle = TempDir::new().unwrap();
        write(&bundle.path().join("opencode.jsonc"), "BUNDLE __LAVIGIE_MCP_TOKEN__");
        let injected = materialize_mcp(bundle.path(), wt.path(), 1, "sekret").unwrap();

        assert!(injected.is_empty(), "should not inject over a user file");
        let got = fs::read_to_string(wt.path().join("opencode.jsonc")).unwrap();
        assert_eq!(got, "USER UNTRACKED");
        assert!(!got.contains("sekret"), "token must not leak into user file: {got}");
    }

    // A user-authored untracked file inside a bundle dotdir must be left alone, and
    // we must NOT drop a `.gitignore="*"` into their dir (which would hide it).
    #[test]
    fn does_not_touch_untracked_user_dotdir_file() {
        let wt = TempDir::new().unwrap();
        init_repo(wt.path());
        write(&wt.path().join(".codex/config.toml"), "USER CODEX");

        let bundle = TempDir::new().unwrap();
        write(&bundle.path().join(".codex/config.toml"), "BUNDLE __LAVIGIE_MCP_TOKEN__");
        let injected = materialize_mcp(bundle.path(), wt.path(), 1, "sekret").unwrap();

        assert!(injected.is_empty());
        assert_eq!(fs::read_to_string(wt.path().join(".codex/config.toml")).unwrap(), "USER CODEX");
        assert!(!wt.path().join(".codex/.gitignore").exists(), "must not hide the user's dotdir");
    }

    // Re-injection into a worktree we already injected into overwrites OUR own
    // (git-excluded) file with a fresh token — the guard must not mistake it for a
    // user file.
    #[test]
    fn reinjection_refreshes_our_own_token() {
        let wt = TempDir::new().unwrap();
        init_repo(wt.path());
        let bundle = TempDir::new().unwrap();
        write(&bundle.path().join("opencode.jsonc"), "Bearer __LAVIGIE_MCP_TOKEN__");
        write(&bundle.path().join(".codex/config.toml"), "Bearer __LAVIGIE_MCP_TOKEN__");

        materialize_mcp(bundle.path(), wt.path(), 1, "token-A").unwrap();
        let injected = materialize_mcp(bundle.path(), wt.path(), 1, "token-B").unwrap();

        assert_eq!(injected, vec![".codex".to_string(), "opencode.jsonc".to_string()]);
        assert!(fs::read_to_string(wt.path().join("opencode.jsonc")).unwrap().contains("token-B"));
        assert!(fs::read_to_string(wt.path().join(".codex/config.toml")).unwrap().contains("token-B"));
        let st = Command::new("git").arg("-C").arg(wt.path()).args(["status","--porcelain"]).output().unwrap();
        assert!(st.stdout.is_empty(), "reinjection left worktree dirty: {}", String::from_utf8_lossy(&st.stdout));
    }
}
