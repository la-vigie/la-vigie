//! Git layer: registering a repo and creating/removing git worktrees by
//! shelling out to the `git` CLI.
//!
//! Scope: this module is the git layer only. No Tauri commands here — those
//! live in `crate::commands`, which calls this module's public functions.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;

use crate::store::Repo;

pub mod worktree_state;
pub use worktree_state::WorktreeAdoption;

// ── Diff / changed-files types ────────────────────────────────────────────────

/// The kind of change a file has undergone vs the base branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Unknown,
}

/// A file and its change-kind relative to the base branch.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    pub path: String,
    pub change: ChangeKind,
}

/// Run `git` with the given args, returning trimmed stdout on success.
/// On non-zero exit, returns an error including stderr.
async fn run_git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .env("LC_ALL", "C")
        .env("LANGUAGE", "C")
        .output()
        .await
        .with_context(|| format!("failed to spawn git {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "git {:?} failed (status {:?}): {}",
            args,
            output.status.code(),
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Validate `path` is inside a git work tree; build a Repo describing it.
/// - name: the repository top-level directory name
/// - default_branch: from `origin/HEAD` if known, else the current branch, else "main"
/// - remote_url: `origin` url if a remote exists, else None
/// - id: a new uuid v4 string
/// Returns Err if `path` is not a git repository.
pub async fn add_repo(path: &Path) -> anyhow::Result<Repo> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("path is not valid UTF-8: {:?}", path))?;

    let toplevel = run_git(&["-C", path_str, "rev-parse", "--show-toplevel"])
        .await
        .context("path is not a git repository")?;

    let name = Path::new(&toplevel)
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("could not derive repo name from toplevel path {}", toplevel))?
        .to_string();

    let default_branch = match run_git(&[
        "-C",
        &toplevel,
        "symbolic-ref",
        "--short",
        "refs/remotes/origin/HEAD",
    ])
    .await
    {
        Ok(branch) => branch
            .strip_prefix("origin/")
            .unwrap_or(&branch)
            .to_string(),
        Err(_) => match run_git(&["-C", &toplevel, "rev-parse", "--abbrev-ref", "HEAD"]).await {
            Ok(branch) if !branch.is_empty() && branch != "HEAD" => branch,
            _ => "main".to_string(),
        },
    };

    let remote_url = run_git(&["-C", &toplevel, "remote", "get-url", "origin"])
        .await
        .ok()
        .filter(|s| !s.is_empty());

    Ok(Repo {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        path: toplevel,
        default_branch,
        remote_url,
        worktree_root: None,
        setup_command: None,
        default_agent: None,
        auto_start_agent: false,
        initial_prompt: None,
        default_model: None,
        sound_settings: None,
        fetch_remote_base: None,
        auto_approve: None,
        in_place_default: false,
    })
}

/// The branch currently checked out in `repo_path`, via
/// `git -C <repo_path> rev-parse --abbrev-ref HEAD`. Returns the branch name
/// (or `"HEAD"` when detached). Used by in-place tasks (TASK-163) that adopt the
/// checkout's current branch rather than creating one.
pub async fn current_branch(repo_path: &Path) -> anyhow::Result<String> {
    let path_str = repo_path
        .to_str()
        .ok_or_else(|| anyhow!("repo_path is not valid UTF-8: {:?}", repo_path))?;
    run_git(&["-C", path_str, "rev-parse", "--abbrev-ref", "HEAD"]).await
}

/// Create and check out a new branch `branch` in the repo's existing checkout:
/// `git -C <repo_path> checkout -b <branch>`. Used by in-place tasks (TASK-163);
/// unlike `create_worktree` this mutates the shared checkout's HEAD. Surfaces
/// git's stderr on failure (e.g. the branch already exists).
pub async fn create_branch_in_place(repo_path: &Path, branch: &str) -> anyhow::Result<()> {
    let path_str = repo_path
        .to_str()
        .ok_or_else(|| anyhow!("repo_path is not valid UTF-8: {:?}", repo_path))?;
    run_git(&["-C", path_str, "checkout", "-b", branch]).await?;
    Ok(())
}

/// `git -C <repo_path> worktree add -b <branch> <worktree_path> <base_branch>`.
/// Creates a new branch `branch` off `base_branch` checked out at `worktree_path`.
pub async fn create_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    branch: &str,
    base_branch: &str,
) -> anyhow::Result<()> {
    let repo_path_str = repo_path
        .to_str()
        .ok_or_else(|| anyhow!("repo_path is not valid UTF-8: {:?}", repo_path))?;
    let worktree_path_str = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("worktree_path is not valid UTF-8: {:?}", worktree_path))?;

    // Branch-safe (TASK-125): if the branch already exists (a leftover from a
    // deleted task, or the branch of a worktree we're reclaiming), check it out
    // into the new worktree instead of `-b` (which would fail "already exists").
    // Reusing preserves the branch's commits; `base_branch` is only the start
    // point for a brand-new branch.
    let branch_exists = ref_exists(repo_path, &format!("refs/heads/{branch}")).await;
    let args: Vec<&str> = if branch_exists {
        vec!["-C", repo_path_str, "worktree", "add", worktree_path_str, branch]
    } else {
        vec!["-C", repo_path_str, "worktree", "add", "-b", branch, worktree_path_str, base_branch]
    };

    run_git(&args).await.with_context(|| {
        if branch_exists {
            format!("failed to create worktree at {worktree_path_str} (reusing branch {branch})")
        } else {
            format!("failed to create worktree at {worktree_path_str} (branch {branch} off {base_branch})")
        }
    })?;

    Ok(())
}

/// Canonicalize a path for comparison, falling back to the path as-given when it
/// doesn't exist (e.g. a registered worktree whose directory was deleted). This
/// resolves macOS `/var` → `/private/var` symlinks so target and git-reported
/// paths compare equal.
fn canonical_or(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// True if `path` is a leftover git-worktree checkout: its `.git` is a *file*
/// containing a `gitdir:` pointer (as git writes for linked worktrees), rather
/// than a directory (a normal repo) or absent (arbitrary user data). Used to tell
/// a reclaimable orphaned worktree apart from a foreign directory we must not
/// clobber.
fn looks_like_orphaned_worktree(path: &Path) -> bool {
    match std::fs::read_to_string(path.join(".git")) {
        Ok(contents) => contents.trim_start().starts_with("gitdir:"),
        // `.git` is a directory (read_to_string errors) or absent → not a worktree.
        Err(_) => false,
    }
}

/// Classify a task's target worktree path (TASK-125): is it free to create, an
/// existing worktree we can adopt, or a conflict to warn about? Runs
/// `git worktree list --porcelain`, canonicalizes both the target and each
/// registered path (macOS `/var` vs `/private/var`), checks disk existence, and
/// defers the decision to the pure `classify_worktree_target`. Never errors — a
/// failed git call is treated as "no registered worktrees".
pub async fn worktree_state(
    repo_path: &Path,
    target_path: &Path,
    branch: &str,
    already_task_owned: bool,
) -> WorktreeAdoption {
    let Some(repo_str) = repo_path.to_str() else {
        return WorktreeAdoption::Conflict {
            reason: format!("repo path is not valid UTF-8: {repo_path:?}"),
        };
    };

    let porcelain = run_git(&["-C", repo_str, "worktree", "list", "--porcelain"])
        .await
        .unwrap_or_default();
    let entries: Vec<worktree_state::WorktreeEntry> =
        worktree_state::parse_worktree_list(&porcelain)
            .into_iter()
            .map(|e| worktree_state::WorktreeEntry {
                path: canonical_or(&e.path),
                registration: e.registration,
            })
            .collect();

    let canon_target = canonical_or(target_path);
    let registration = worktree_state::match_registration(&entries, &canon_target);
    let path_exists = target_path.exists();
    // Only relevant when the path exists but git doesn't track it: is it a
    // reclaimable leftover worktree, or foreign data we must not touch?
    let is_orphaned_worktree =
        path_exists && registration.is_none() && looks_like_orphaned_worktree(target_path);

    worktree_state::classify_worktree_target(
        registration.as_ref(),
        path_exists,
        is_orphaned_worktree,
        already_task_owned,
        branch,
        &target_path.to_string_lossy(),
    )
}

/// `git -C <repo_path> fetch <remote> <base>:refs/remotes/<remote>/<base>`.
/// The explicit refspec forces the remote-tracking ref (`refs/remotes/<remote>/<base>`)
/// to update. A plain `git fetch <remote> <base>` can leave that ref stale on
/// clones without a configured fetch refspec for the branch (TASK-144). Returns
/// the error (including git's stderr) on failure — callers decide whether to fall
/// back to the local base.
pub async fn fetch(repo_path: &Path, remote: &str, base: &str) -> anyhow::Result<()> {
    let repo_path_str = repo_path
        .to_str()
        .ok_or_else(|| anyhow!("repo_path is not valid UTF-8: {:?}", repo_path))?;

    let refspec = format!("{base}:refs/remotes/{remote}/{base}");
    run_git(&["-C", repo_path_str, "fetch", remote, &refspec])
        .await
        .with_context(|| format!("failed to fetch {remote} {base}"))?;

    Ok(())
}

/// True if `refname` resolves in the repo/worktree at `dir`
/// (`git rev-parse --verify --quiet <refname>` exits 0). Used to detect whether
/// a remote-tracking ref like `origin/main` is present before comparing against
/// it. Any error (missing ref, bad dir) is treated as "not present".
pub async fn ref_exists(dir: &Path, refname: &str) -> bool {
    let Some(dir_str) = dir.to_str() else {
        return false;
    };
    run_git(&["-C", dir_str, "rev-parse", "--verify", "--quiet", refname])
        .await
        .is_ok()
}

/// Delete a local branch. `force=true` uses `-D` (delete even if unmerged), else `-d`.
/// NOTE: a branch checked out in a worktree cannot be deleted — callers must remove the
/// worktree first.
pub async fn delete_branch(repo_path: &Path, branch: &str, force: bool) -> anyhow::Result<()> {
    let repo_path_str = repo_path
        .to_str()
        .ok_or_else(|| anyhow!("repo_path is not valid UTF-8: {:?}", repo_path))?;

    let flag = if force { "-D" } else { "-d" };

    run_git(&["-C", repo_path_str, "branch", flag, branch])
        .await
        .with_context(|| format!("failed to delete branch {branch} in {repo_path_str}"))?;

    Ok(())
}

/// List the repo's local branch names, sorted alphabetically.
/// Uses `git for-each-ref refs/heads` so the output is one clean short name per
/// line (no `*`/whitespace decoration that `git branch` adds).
pub async fn list_branches(repo_path: &Path) -> anyhow::Result<Vec<String>> {
    let repo_path_str = repo_path
        .to_str()
        .ok_or_else(|| anyhow!("repo_path is not valid UTF-8: {:?}", repo_path))?;

    let out = run_git(&[
        "-C",
        repo_path_str,
        "for-each-ref",
        "--format=%(refname:short)",
        "--sort=refname",
        "refs/heads",
    ])
    .await
    .with_context(|| format!("failed to list branches in {repo_path_str}"))?;

    Ok(out.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect())
}

/// `git -C <repo_path> worktree remove [--force] <worktree_path>`.
///
/// Idempotent: if the worktree is already absent or deregistered (admin entry
/// gone but directory still on disk), git returns "is not a working tree". We
/// handle that by running `git worktree prune` and removing the directory
/// ourselves so cleanup still completes cleanly.
pub async fn remove_worktree(
    repo_path: &Path,
    worktree_path: &Path,
    force: bool,
) -> anyhow::Result<()> {
    let repo_path_str = repo_path
        .to_str()
        .ok_or_else(|| anyhow!("repo_path is not valid UTF-8: {:?}", repo_path))?;
    let worktree_path_str = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("worktree_path is not valid UTF-8: {:?}", worktree_path))?;

    let mut args = vec!["-C", repo_path_str, "worktree", "remove"];
    if force {
        args.push("--force");
    }
    args.push(worktree_path_str);

    match run_git(&args).await {
        Ok(_) => Ok(()),
        Err(e) => {
            // The worktree may be deregistered (admin entry gone) or already
            // absent: `git worktree remove` then fails with "is not a working
            // tree". Fall back to pruning stale admin entries + best-effort
            // directory removal so cleanup still completes. An already-absent
            // worktree is treated as success.
            let msg = e.to_string();
            if msg.contains("is not a working tree") || msg.contains("No such file or directory") {
                let _ = run_git(&["-C", repo_path_str, "worktree", "prune"]).await;
                if worktree_path.exists() {
                    std::fs::remove_dir_all(worktree_path).with_context(|| {
                        format!("failed to rm deregistered worktree dir {}", worktree_path_str)
                    })?;
                    // Prune again so the now-removed dir's admin entry (if any) is cleared.
                    let _ = run_git(&["-C", repo_path_str, "worktree", "prune"]).await;
                }
                Ok(())
            } else {
                Err(e).with_context(|| {
                    format!("failed to remove worktree at {}", worktree_path_str)
                })
            }
        }
    }
}

// ── Diff helpers ──────────────────────────────────────────────────────────────

/// Resolve the merge-base commit between `base_branch` and the worktree's HEAD.
/// This is the point where the branch diverged from the base. Diffing against
/// it (three-dot / GitHub-PR semantics) shows only the branch's own changes and
/// is unaffected by the base branch advancing — no rebase required.
async fn merge_base(wt: &str, base_branch: &str) -> Result<String> {
    run_git(&["-C", wt, "merge-base", base_branch, "HEAD"])
        .await
        .with_context(|| format!("failed to find merge-base with {base_branch}"))
}

/// Unified diff of the worktree vs the merge-base with `base_branch`.
/// Runs `git -C <worktree> diff <merge-base>` — returns raw unified diff text
/// (may be empty when there are no changes). Using the merge-base (rather than
/// the base tip) means base-only commits don't appear as reverse changes.
pub async fn diff_against_base(worktree_path: &Path, base_branch: &str) -> Result<String> {
    let wt = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("worktree_path is not valid UTF-8: {:?}", worktree_path))?;

    let base = merge_base(wt, base_branch).await?;

    // run_git trims the output; for a diff we want the raw bytes including
    // trailing newlines so the caller gets a faithful unified diff.
    let output = Command::new("git")
        .args(["-C", wt, "diff", &base])
        .output()
        .await
        .with_context(|| format!("failed to spawn git diff against {base}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "git diff {} failed (status {:?}): {}",
            base,
            output.status.code(),
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Changed files vs the merge-base with `base_branch`, parsed from
/// `git diff --name-status`. Returns an empty vec when there are no changes.
/// Note: untracked (never-staged) files are not shown by `git diff`; that is an
/// accepted M4 limitation.
pub async fn changed_files(worktree_path: &Path, base_branch: &str) -> Result<Vec<FileChange>> {
    let wt = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("worktree_path is not valid UTF-8: {:?}", worktree_path))?;

    let base = merge_base(wt, base_branch).await?;

    let raw = run_git(&["-C", wt, "diff", "--name-status", &base]).await?;

    if raw.is_empty() {
        return Ok(vec![]);
    }

    let mut changes = Vec::new();
    for line in raw.lines() {
        let parts: Vec<&str> = line.splitn(3, '\t').collect();
        if parts.is_empty() {
            continue;
        }
        let status = parts[0];
        // Rename/copy lines: `R100\told\tnew` — use the NEW path.
        let path = if parts.len() >= 3 {
            parts[2].to_string()
        } else if parts.len() >= 2 {
            parts[1].to_string()
        } else {
            continue;
        };

        let change = if status.starts_with('R') {
            ChangeKind::Renamed
        } else if status.starts_with('C') {
            ChangeKind::Copied
        } else {
            match status {
                "A" => ChangeKind::Added,
                "M" => ChangeKind::Modified,
                "D" => ChangeKind::Deleted,
                "T" => ChangeKind::TypeChanged,
                _ => ChangeKind::Unknown,
            }
        };

        changes.push(FileChange { path, change });
    }

    Ok(changes)
}

/// Untracked (never-staged), non-ignored files in the worktree, as relative
/// paths. Runs `git -C <wt> ls-files --others --exclude-standard`, so
/// `.gitignore`d files are excluded. Empty vec when there are none.
pub async fn untracked_files(worktree_path: &Path) -> Result<Vec<String>> {
    let wt = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("worktree_path is not valid UTF-8: {:?}", worktree_path))?;

    let raw = run_git(&["-C", wt, "ls-files", "--others", "--exclude-standard"]).await?;
    if raw.is_empty() {
        return Ok(vec![]);
    }
    Ok(raw.lines().map(|l| l.to_string()).collect())
}

/// True if the worktree has any uncommitted changes or untracked files
/// (`git status --porcelain` produces at least one line).
pub async fn working_tree_dirty(worktree: &Path) -> Result<bool> {
    let wt = worktree
        .to_str()
        .ok_or_else(|| anyhow!("worktree_path is not valid UTF-8: {:?}", worktree))?;
    let out = run_git(&["-C", wt, "status", "--porcelain"]).await?;
    Ok(!out.trim().is_empty())
}

/// True if `HEAD` has any commits not reachable from `base` (`git rev-list
/// <base>..HEAD` is non-empty). Note: after a squash-merge the original commits
/// never appear in base, so this stays true — callers combine it with the PR
/// merged-state to decide "already merged".
pub async fn commits_ahead_of_base(worktree: &Path, base: &str) -> Result<bool> {
    let wt = worktree
        .to_str()
        .ok_or_else(|| anyhow!("worktree_path is not valid UTF-8: {:?}", worktree))?;
    let range = format!("{base}..HEAD");
    let out = run_git(&["-C", wt, "rev-list", &range]).await?;
    Ok(!out.trim().is_empty())
}

/// Synthesized unified diff (all additions) for each untracked, non-ignored
/// file, via `git -C <wt> diff --no-index /dev/null <file>`, concatenated.
/// `--no-index` exits 1 when the files differ (normal for a new file), so exit
/// codes 0 and 1 are treated as success; any other status is an error. Binary
/// files render as git's "Binary files … differ"; empty files yield no body.
pub async fn diff_untracked(worktree_path: &Path) -> Result<String> {
    let wt = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("worktree_path is not valid UTF-8: {:?}", worktree_path))?;

    let files = untracked_files(worktree_path).await?;
    let mut out = String::new();
    for path in &files {
        let output = Command::new("git")
            .args(["-C", wt, "diff", "--no-index", "/dev/null", path])
            .output()
            .await
            .with_context(|| format!("failed to spawn git diff --no-index for {path}"))?;

        match output.status.code() {
            Some(0) | Some(1) => out.push_str(&String::from_utf8_lossy(&output.stdout)),
            other => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(anyhow!(
                    "git diff --no-index for {path} failed (status {other:?}): {}",
                    stderr.trim()
                ));
            }
        }
    }
    Ok(out)
}

/// Stage the given paths: `git -C <worktree> add -- <paths...>`.
pub async fn stage(worktree_path: &Path, paths: &[String]) -> Result<()> {
    let wt = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("worktree_path is not valid UTF-8: {:?}", worktree_path))?;

    if paths.is_empty() {
        return Ok(());
    }

    let mut args = vec!["-C", wt, "add", "--"];
    for p in paths {
        args.push(p.as_str());
    }

    run_git(&args)
        .await
        .with_context(|| format!("failed to stage paths in {wt}"))?;

    Ok(())
}

/// Commit staged changes: `git -C <worktree> commit -m <message>`.
pub async fn commit(worktree_path: &Path, message: &str) -> Result<()> {
    let wt = worktree_path
        .to_str()
        .ok_or_else(|| anyhow!("worktree_path is not valid UTF-8: {:?}", worktree_path))?;

    run_git(&["-C", wt, "commit", "-m", message])
        .await
        .with_context(|| format!("failed to commit in {wt}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Build a throwaway git repo deterministically at a fresh TempDir.
    fn init_test_repo() -> TempDir {
        let dir = TempDir::new().expect("create temp dir");
        let path = dir.path();

        Command::new("git")
            .args(["init", "-b", "main"])
            .arg(path)
            .output()
            .expect("git init");
        Command::new("git")
            .args(["-C", path.to_str().unwrap(), "config", "user.email", "test@example.com"])
            .output()
            .expect("git config user.email");
        Command::new("git")
            .args(["-C", path.to_str().unwrap(), "config", "user.name", "Test"])
            .output()
            .expect("git config user.name");
        std::fs::write(path.join("README.md"), "hi\n").expect("write README");
        Command::new("git")
            .args(["-C", path.to_str().unwrap(), "add", "-A"])
            .output()
            .expect("git add");
        Command::new("git")
            .args(["-C", path.to_str().unwrap(), "commit", "-m", "init"])
            .output()
            .expect("git commit");

        dir
    }

    #[tokio::test]
    async fn add_repo_on_plain_repo_has_expected_fields() {
        let dir = init_test_repo();
        let repo = add_repo(dir.path()).await.expect("add_repo should succeed");

        let expected_name = dir.path().file_name().unwrap().to_str().unwrap();
        assert_eq!(repo.name, expected_name);
        assert_eq!(repo.default_branch, "main");
        assert_eq!(repo.remote_url, None);
    }

    #[tokio::test]
    async fn add_repo_with_origin_remote_captures_remote_url() {
        let dir = init_test_repo();
        let url = "https://example.com/x.git";
        Command::new("git")
            .args(["-C", dir.path().to_str().unwrap(), "remote", "add", "origin", url])
            .output()
            .expect("git remote add");

        let repo = add_repo(dir.path()).await.expect("add_repo should succeed");

        assert_eq!(repo.remote_url, Some(url.to_string()));
    }

    #[tokio::test]
    async fn add_repo_on_non_git_dir_errors() {
        let dir = TempDir::new().expect("create temp dir");

        let result = add_repo(dir.path()).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn create_worktree_checks_out_new_branch_off_base() {
        let dir = init_test_repo();
        let worktree_parent = TempDir::new().expect("create temp dir");
        let worktree_path = worktree_parent.path().join("wt");

        create_worktree(dir.path(), &worktree_path, "feature-x", "main")
            .await
            .expect("create_worktree should succeed");

        assert!(worktree_path.exists());

        let output = Command::new("git")
            .args(["-C", worktree_path.to_str().unwrap(), "rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .expect("git rev-parse");
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        assert_eq!(branch, "feature-x");
    }

    #[tokio::test]
    async fn current_branch_reports_the_checked_out_branch() {
        let dir = tempfile::TempDir::new().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-C", dir.path().to_str().unwrap()])
                .args(args)
                .output()
                .expect("git");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        run(&["commit", "-q", "--allow-empty", "-m", "init"]);

        let branch = current_branch(dir.path()).await.expect("current_branch");
        assert_eq!(branch, "main");
    }

    #[tokio::test]
    async fn create_branch_in_place_switches_and_rejects_duplicates() {
        let dir = tempfile::TempDir::new().unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(["-C", dir.path().to_str().unwrap()])
                .args(args)
                .output()
                .expect("git");
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        run(&["commit", "-q", "--allow-empty", "-m", "init"]);

        create_branch_in_place(dir.path(), "feature-x")
            .await
            .expect("create_branch_in_place");
        assert_eq!(current_branch(dir.path()).await.unwrap(), "feature-x");

        // Creating the same branch again fails (git: already exists).
        let err = create_branch_in_place(dir.path(), "feature-x").await;
        assert!(err.is_err(), "duplicate branch must surface an error");
    }

    // ── worktree_state (TASK-125): async glue over the pure classifier ───────────
    // These exercise the real `git worktree list` + macOS /var→/private/var
    // canonicalization that the pure unit tests in `worktree_state` can't.

    #[tokio::test]
    async fn worktree_state_vacant_for_a_fresh_path() {
        let dir = init_test_repo();
        let parent = TempDir::new().expect("create temp dir");
        let target = parent.path().join("nope");

        let state = worktree_state(dir.path(), &target, "feature-x", false).await;
        assert_eq!(state, WorktreeAdoption::Vacant);
    }

    #[tokio::test]
    async fn worktree_state_adopts_a_matching_existing_worktree() {
        let dir = init_test_repo();
        let parent = TempDir::new().expect("create temp dir");
        let target = parent.path().join("wt");
        create_worktree(dir.path(), &target, "feature-x", "main")
            .await
            .expect("create_worktree should succeed");

        // Same branch at the same path → adopt (this is the leftover-worktree case).
        let state = worktree_state(dir.path(), &target, "feature-x", false).await;
        assert_eq!(state, WorktreeAdoption::Adopt);
    }

    #[tokio::test]
    async fn worktree_state_conflicts_on_branch_mismatch() {
        let dir = init_test_repo();
        let parent = TempDir::new().expect("create temp dir");
        let target = parent.path().join("wt");
        create_worktree(dir.path(), &target, "feature-x", "main")
            .await
            .expect("create_worktree should succeed");

        // A different intended branch at an occupied path → conflict, not adopt.
        let state = worktree_state(dir.path(), &target, "feature-other", false).await;
        assert!(matches!(state, WorktreeAdoption::Conflict { .. }), "got: {state:?}");
    }

    #[tokio::test]
    async fn worktree_state_conflicts_on_bare_dir_not_a_worktree() {
        let dir = init_test_repo();
        let parent = TempDir::new().expect("create temp dir");
        let target = parent.path().join("plain");
        std::fs::create_dir_all(&target).expect("mkdir");

        // A plain directory that isn't a registered worktree → conflict.
        let state = worktree_state(dir.path(), &target, "feature-x", false).await;
        assert!(matches!(state, WorktreeAdoption::Conflict { .. }), "got: {state:?}");
    }

    #[tokio::test]
    async fn worktree_state_conflicts_when_already_task_owned() {
        let dir = init_test_repo();
        let parent = TempDir::new().expect("create temp dir");
        let target = parent.path().join("wt");
        create_worktree(dir.path(), &target, "feature-x", "main")
            .await
            .expect("create_worktree should succeed");

        // Even a matching worktree is a conflict if a live task already owns it.
        let state = worktree_state(dir.path(), &target, "feature-x", true).await;
        assert!(matches!(state, WorktreeAdoption::Conflict { .. }), "got: {state:?}");
    }

    #[tokio::test]
    async fn worktree_state_reclaims_an_orphaned_worktree() {
        let dir = init_test_repo();
        let parent = TempDir::new().expect("create temp dir");
        let target = parent.path().join("wt");
        create_worktree(dir.path(), &target, "feature-x", "main")
            .await
            .expect("create_worktree should succeed");

        // Orphan it: delete the admin dir but keep the checkout + its `.git`
        // pointer (exactly the user's stuck-`testing` state). git now omits it
        // from `worktree list` and rejects it.
        std::fs::remove_dir_all(dir.path().join(".git/worktrees/wt"))
            .expect("remove admin dir");

        let state = worktree_state(dir.path(), &target, "feature-x", false).await;
        assert!(matches!(state, WorktreeAdoption::Reclaim { .. }), "got: {state:?}");
    }

    #[tokio::test]
    async fn create_worktree_reuses_an_existing_branch() {
        let dir = init_test_repo();
        // A leftover branch with no worktree (deleted-task residue).
        Command::new("git")
            .args(["-C", dir.path().to_str().unwrap(), "branch", "feature-z"])
            .output()
            .expect("git branch");

        let parent = TempDir::new().expect("create temp dir");
        let target = parent.path().join("wt");
        // Would fail with `-b` ("branch already exists"); branch-safe path reuses it.
        create_worktree(dir.path(), &target, "feature-z", "main")
            .await
            .expect("create_worktree should reuse the existing branch");

        assert!(target.exists());
        let output = Command::new("git")
            .args(["-C", target.to_str().unwrap(), "rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .expect("git rev-parse");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "feature-z");
    }

    #[tokio::test]
    async fn remove_worktree_deletes_the_worktree() {
        let dir = init_test_repo();
        let worktree_parent = TempDir::new().expect("create temp dir");
        let worktree_path = worktree_parent.path().join("wt");

        create_worktree(dir.path(), &worktree_path, "feature-y", "main")
            .await
            .expect("create_worktree should succeed");

        remove_worktree(dir.path(), &worktree_path, false)
            .await
            .expect("remove_worktree should succeed");

        assert!(!worktree_path.exists());

        let output = Command::new("git")
            .args(["-C", dir.path().to_str().unwrap(), "worktree", "list"])
            .output()
            .expect("git worktree list");
        let listing = String::from_utf8_lossy(&output.stdout);
        assert!(!listing.contains(worktree_path.to_str().unwrap()));
    }

    #[tokio::test]
    async fn remove_worktree_succeeds_when_dir_present_but_deregistered() {
        let dir = init_test_repo();
        let worktree_parent = TempDir::new().expect("create temp dir");
        let worktree_path = worktree_parent.path().join("wt");

        create_worktree(dir.path(), &worktree_path, "feature-z", "main")
            .await
            .expect("create_worktree should succeed");

        // Simulate a deregistered worktree: remove git's admin entry so the dir
        // is still on disk but `git worktree remove` would fail with
        // "is not a working tree".
        let admin = dir.path().join(".git/worktrees/wt");
        std::fs::remove_dir_all(&admin).expect("remove worktree admin entry");

        // The dir is still present.
        assert!(worktree_path.exists());

        // Idempotent remove must still succeed and clean the directory up.
        remove_worktree(dir.path(), &worktree_path, true)
            .await
            .expect("remove_worktree should treat a deregistered worktree as success");

        assert!(!worktree_path.exists());
    }

    #[tokio::test]
    async fn remove_worktree_succeeds_when_worktree_already_absent() {
        let dir = init_test_repo();
        let worktree_parent = TempDir::new().expect("create temp dir");
        let worktree_path = worktree_parent.path().join("wt");

        create_worktree(dir.path(), &worktree_path, "feature-gone", "main")
            .await
            .expect("create_worktree should succeed");

        // First removal: removes the worktree and its admin entry normally.
        remove_worktree(dir.path(), &worktree_path, true)
            .await
            .expect("first remove_worktree should succeed");

        assert!(!worktree_path.exists(), "worktree dir should be gone after first removal");

        // Second removal: both the admin entry and the directory are already
        // gone — the idempotent fallback must still return Ok.
        remove_worktree(dir.path(), &worktree_path, true)
            .await
            .expect("second remove_worktree should succeed (already absent is idempotent)");
    }

    #[tokio::test]
    async fn list_branches_returns_local_branches_sorted() {
        let dir = init_test_repo();
        // init_test_repo leaves us on `main`; add two more branches.
        for b in ["zeta", "alpha"] {
            Command::new("git")
                .args(["-C", dir.path().to_str().unwrap(), "branch", b])
                .output()
                .expect("git branch");
        }

        let branches = list_branches(dir.path()).await.expect("list_branches should succeed");

        assert_eq!(branches, vec!["alpha", "main", "zeta"]);
    }

    #[tokio::test]
    async fn list_branches_on_non_git_dir_errors() {
        let dir = TempDir::new().expect("create temp dir");
        assert!(list_branches(dir.path()).await.is_err());
    }

    // ── Helpers for diff/stage/commit tests ───────────────────────────────────

    /// Create a worktree from `base_repo` on a new branch `branch_name`.
    fn create_test_worktree(base_repo: &TempDir, branch_name: &str) -> TempDir {
        let wt_dir = TempDir::new().expect("create worktree temp dir");
        let wt_path = wt_dir.path().join("wt");

        Command::new("git")
            .args([
                "-C",
                base_repo.path().to_str().unwrap(),
                "worktree",
                "add",
                "-b",
                branch_name,
                wt_path.to_str().unwrap(),
                "main",
            ])
            .output()
            .expect("git worktree add");

        // Box the two TempDirs together so the base repo lives as long as the
        // worktree.  We only need the worktree path; we leak the base_repo
        // lifetime by returning a fresh TempDir whose path IS the worktree.
        // Instead, we create a new temp dir that wraps the wt sub-path.
        //
        // Actually: return just wt_dir; the worktree sub-dir is wt_dir/wt.
        // Callers use `wt_dir.path().join("wt")` to get the real path.
        wt_dir
    }

    // ── diff_against_base tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn diff_against_base_includes_uncommitted_modification() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-mod");
        let wt = wt_parent.path().join("wt");

        // Modify a tracked file (uncommitted)
        std::fs::write(wt.join("README.md"), "hi\nextra line\n").expect("modify README");

        let diff = diff_against_base(&wt, "main").await.expect("diff should succeed");

        assert!(
            diff.contains("extra line"),
            "diff should contain the new line; got:\n{diff}"
        );
    }

    #[tokio::test]
    async fn diff_against_base_is_empty_when_no_changes() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-clean");
        let wt = wt_parent.path().join("wt");

        let diff = diff_against_base(&wt, "main").await.expect("diff should succeed");

        assert!(diff.is_empty(), "diff should be empty on clean worktree; got:\n{diff}");
    }

    #[tokio::test]
    async fn diff_against_base_includes_committed_branch_work() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-committed");
        let wt = wt_parent.path().join("wt");

        // Add a new file, stage and commit it on the branch
        std::fs::write(wt.join("new_file.txt"), "new content\n").expect("write new file");
        Command::new("git")
            .args(["-C", wt.to_str().unwrap(), "add", "new_file.txt"])
            .output()
            .expect("git add");
        Command::new("git")
            .args(["-C", wt.to_str().unwrap(), "commit", "-m", "add new file"])
            .output()
            .expect("git commit");

        let diff = diff_against_base(&wt, "main").await.expect("diff should succeed");

        assert!(
            diff.contains("new content"),
            "diff should show committed branch work; got:\n{diff}"
        );
    }

    #[tokio::test]
    async fn diff_against_base_excludes_base_only_commits_after_base_advances() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-diverge");
        let wt = wt_parent.path().join("wt");

        // Commit the branch's own work.
        std::fs::write(wt.join("branch_file.txt"), "branch work\n").expect("write branch file");
        Command::new("git")
            .args(["-C", wt.to_str().unwrap(), "add", "branch_file.txt"])
            .output()
            .expect("git add");
        Command::new("git")
            .args(["-C", wt.to_str().unwrap(), "commit", "-m", "branch commit"])
            .output()
            .expect("git commit");

        // Advance the base branch past the divergence point (no rebase).
        std::fs::write(base.path().join("base_file.txt"), "base advanced\n")
            .expect("write base file");
        Command::new("git")
            .args(["-C", base.path().to_str().unwrap(), "add", "base_file.txt"])
            .output()
            .expect("git add");
        Command::new("git")
            .args(["-C", base.path().to_str().unwrap(), "commit", "-m", "base advanced"])
            .output()
            .expect("git commit");

        let diff = diff_against_base(&wt, "main").await.expect("diff should succeed");

        assert!(
            diff.contains("branch work"),
            "diff should show the branch's own work; got:\n{diff}"
        );
        assert!(
            !diff.contains("base advanced") && !diff.contains("base_file.txt"),
            "diff must not contain base-only commits (reverse/removal noise); got:\n{diff}"
        );
    }

    // ── changed_files tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn changed_files_reports_modified_uncommitted_file() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-cf-mod");
        let wt = wt_parent.path().join("wt");

        std::fs::write(wt.join("README.md"), "hi\nextra\n").expect("modify README");

        let changes = changed_files(&wt, "main").await.expect("changed_files should succeed");

        assert_eq!(changes.len(), 1, "expected 1 change; got {changes:?}");
        assert_eq!(changes[0].path, "README.md");
        assert_eq!(changes[0].change, ChangeKind::Modified);
    }

    #[tokio::test]
    async fn changed_files_reports_added_committed_file() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-cf-add");
        let wt = wt_parent.path().join("wt");

        std::fs::write(wt.join("added.txt"), "I am new\n").expect("write added.txt");
        Command::new("git")
            .args(["-C", wt.to_str().unwrap(), "add", "added.txt"])
            .output()
            .expect("git add");
        Command::new("git")
            .args(["-C", wt.to_str().unwrap(), "commit", "-m", "add file"])
            .output()
            .expect("git commit");

        let changes = changed_files(&wt, "main").await.expect("changed_files should succeed");

        let added: Vec<_> = changes
            .iter()
            .filter(|c| c.path == "added.txt")
            .collect();
        assert_eq!(added.len(), 1, "expected added.txt in changes; got {changes:?}");
        assert_eq!(added[0].change, ChangeKind::Added);
    }

    #[tokio::test]
    async fn changed_files_reports_deleted_file() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-cf-del");
        let wt = wt_parent.path().join("wt");

        std::fs::remove_file(wt.join("README.md")).expect("delete README");

        let changes = changed_files(&wt, "main").await.expect("changed_files should succeed");

        assert_eq!(changes.len(), 1, "expected 1 change; got {changes:?}");
        assert_eq!(changes[0].path, "README.md");
        assert_eq!(changes[0].change, ChangeKind::Deleted);
    }

    #[tokio::test]
    async fn changed_files_empty_when_no_changes() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-cf-empty");
        let wt = wt_parent.path().join("wt");

        let changes = changed_files(&wt, "main").await.expect("changed_files should succeed");

        assert!(changes.is_empty(), "expected no changes; got {changes:?}");
    }

    #[tokio::test]
    async fn changed_files_excludes_base_only_commits_after_base_advances() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-cf-diverge");
        let wt = wt_parent.path().join("wt");

        // Commit the branch's own work.
        std::fs::write(wt.join("branch_file.txt"), "branch work\n").expect("write branch file");
        Command::new("git")
            .args(["-C", wt.to_str().unwrap(), "add", "branch_file.txt"])
            .output()
            .expect("git add");
        Command::new("git")
            .args(["-C", wt.to_str().unwrap(), "commit", "-m", "branch commit"])
            .output()
            .expect("git commit");

        // Advance the base branch past the divergence point (no rebase).
        std::fs::write(base.path().join("base_file.txt"), "base advanced\n")
            .expect("write base file");
        Command::new("git")
            .args(["-C", base.path().to_str().unwrap(), "add", "base_file.txt"])
            .output()
            .expect("git add");
        Command::new("git")
            .args(["-C", base.path().to_str().unwrap(), "commit", "-m", "base advanced"])
            .output()
            .expect("git commit");

        let changes = changed_files(&wt, "main").await.expect("changed_files should succeed");

        let paths: Vec<&str> = changes.iter().map(|c| c.path.as_str()).collect();
        assert!(
            paths.contains(&"branch_file.txt"),
            "changed_files should list the branch's own file; got {changes:?}"
        );
        assert!(
            !paths.contains(&"base_file.txt"),
            "changed_files must not list base-only files (reverse/removal noise); got {changes:?}"
        );
    }

    // ── stage + commit tests ───────────────────────────────────────────────────

    #[tokio::test]
    async fn stage_and_commit_records_commit_with_message() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-commit");
        let wt = wt_parent.path().join("wt");

        std::fs::write(wt.join("staged.txt"), "staged content\n").expect("write staged.txt");

        // Stage via our function
        stage(&wt, &["staged.txt".to_string()]).await.expect("stage should succeed");

        // Commit via our function
        let msg = "my test commit";
        commit(&wt, msg).await.expect("commit should succeed");

        // Verify with real git: subject of last commit
        let git_log = Command::new("git")
            .args(["-C", wt.to_str().unwrap(), "log", "-1", "--pretty=%s"])
            .output()
            .expect("git log");
        let subject = String::from_utf8_lossy(&git_log.stdout).trim().to_string();
        assert_eq!(subject, msg, "commit subject should match message");

        // Verify nothing staged after commit
        let status = Command::new("git")
            .args(["-C", wt.to_str().unwrap(), "status", "--porcelain"])
            .output()
            .expect("git status");
        let status_str = String::from_utf8_lossy(&status.stdout).to_string();
        assert!(
            !status_str.contains("staged.txt"),
            "staged.txt should not appear in status after commit; got:\n{status_str}"
        );
    }

    #[tokio::test]
    async fn stage_empty_paths_is_a_noop() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-stage-noop");
        let wt = wt_parent.path().join("wt");

        // Should not error
        stage(&wt, &[]).await.expect("stage with empty paths should succeed");
    }

    // ── delete_branch tests ───────────────────────────────────────────────────

    /// Helper: check whether `branch` appears in `git branch --list <branch>`.
    fn branch_exists(repo: &Path, branch: &str) -> bool {
        let output = Command::new("git")
            .args(["-C", repo.to_str().unwrap(), "branch", "--list", branch])
            .output()
            .expect("git branch --list");
        !String::from_utf8_lossy(&output.stdout).trim().is_empty()
    }

    #[tokio::test]
    async fn delete_branch_force_removes_unmerged_branch() {
        let dir = init_test_repo();
        // Create a branch (does not need to be checked out)
        Command::new("git")
            .args(["-C", dir.path().to_str().unwrap(), "branch", "feature"])
            .output()
            .expect("git branch feature");

        assert!(branch_exists(dir.path(), "feature"), "branch should exist before deletion");

        delete_branch(dir.path(), "feature", true)
            .await
            .expect("delete_branch should succeed");

        assert!(!branch_exists(dir.path(), "feature"), "branch should be gone after deletion");
    }

    #[tokio::test]
    async fn delete_branch_on_nonexistent_branch_errors() {
        let dir = init_test_repo();

        let result = delete_branch(dir.path(), "does-not-exist", true).await;

        assert!(result.is_err(), "deleting a nonexistent branch should return Err");
    }

    // ── finish_task lifecycle (git-level) tests ───────────────────────────────
    //
    // These test the git-level sequence that finish_task performs:
    //   discard: remove_worktree → delete_branch  (worktree gone, branch gone)
    //   keep:    remove_worktree only             (worktree gone, branch survives)

    #[tokio::test]
    async fn finish_task_discard_removes_worktree_and_branch() {
        let dir = init_test_repo();
        let worktree_parent = TempDir::new().expect("create temp dir");
        let worktree_path = worktree_parent.path().join("wt");

        create_worktree(dir.path(), &worktree_path, "wt", "main")
            .await
            .expect("create_worktree should succeed");

        assert!(worktree_path.exists(), "worktree path should exist after creation");
        assert!(branch_exists(dir.path(), "wt"), "branch should exist after worktree creation");

        // discard sequence: remove worktree, then delete branch
        remove_worktree(dir.path(), &worktree_path, true)
            .await
            .expect("remove_worktree should succeed");
        delete_branch(dir.path(), "wt", true)
            .await
            .expect("delete_branch should succeed after worktree removal");

        assert!(!worktree_path.exists(), "worktree path should be gone");
        assert!(!branch_exists(dir.path(), "wt"), "branch should be gone in discard mode");
    }

    // ── untracked_files tests ─────────────────────────────────────────────────

    #[tokio::test]
    async fn untracked_files_lists_new_file_and_excludes_gitignored() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-untracked-list");
        let wt = wt_parent.path().join("wt");

        std::fs::write(wt.join("visible.txt"), "new file\n").expect("write visible.txt");
        std::fs::write(wt.join(".gitignore"), "ignored.txt\n").expect("write .gitignore");
        std::fs::write(wt.join("ignored.txt"), "secret\n").expect("write ignored.txt");

        let files = untracked_files(&wt).await.expect("untracked_files should succeed");

        assert!(files.iter().any(|p| p == "visible.txt"), "expected visible.txt; got {files:?}");
        assert!(!files.iter().any(|p| p == "ignored.txt"), "ignored.txt must be excluded; got {files:?}");
    }

    #[tokio::test]
    async fn untracked_files_empty_when_worktree_clean() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-untracked-clean");
        let wt = wt_parent.path().join("wt");

        let files = untracked_files(&wt).await.expect("untracked_files should succeed");
        assert!(files.is_empty(), "expected no untracked files; got {files:?}");
    }

    // ── diff_untracked tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn diff_untracked_shows_added_body_for_new_text_file() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-untracked-diff");
        let wt = wt_parent.path().join("wt");

        std::fs::write(wt.join("new.txt"), "alpha\nbeta\n").expect("write new.txt");

        let diff = diff_untracked(&wt).await.expect("diff_untracked should succeed");

        assert!(diff.contains("new.txt"), "diff should reference new.txt; got:\n{diff}");
        assert!(diff.contains("+alpha"), "diff should show added line +alpha; got:\n{diff}");
        assert!(diff.contains("+beta"), "diff should show added line +beta; got:\n{diff}");
    }

    #[tokio::test]
    async fn diff_untracked_marks_binary_file() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-untracked-binary");
        let wt = wt_parent.path().join("wt");

        // NUL bytes make git treat the file as binary.
        std::fs::write(wt.join("blob.bin"), [0u8, 159, 146, 150, 0, 1, 2]).expect("write blob.bin");

        let diff = diff_untracked(&wt).await.expect("diff_untracked should succeed");
        assert!(diff.contains("Binary files"), "binary file should be marked; got:\n{diff}");
    }

    #[tokio::test]
    async fn diff_untracked_empty_file_has_no_body() {
        let base = init_test_repo();
        let wt_parent = create_test_worktree(&base, "branch-untracked-empty");
        let wt = wt_parent.path().join("wt");

        std::fs::write(wt.join("empty.txt"), "").expect("write empty.txt");

        let diff = diff_untracked(&wt).await.expect("diff_untracked should succeed");
        // An empty new file is identical to /dev/null → no diff body emitted.
        assert!(!diff.contains("+++"), "empty file should produce no added body; got:\n{diff}");
    }

    #[tokio::test]
    async fn finish_task_keep_removes_worktree_but_preserves_branch() {
        let dir = init_test_repo();
        let worktree_parent = TempDir::new().expect("create temp dir");
        let worktree_path = worktree_parent.path().join("wt");

        create_worktree(dir.path(), &worktree_path, "wt-keep", "main")
            .await
            .expect("create_worktree should succeed");

        assert!(worktree_path.exists(), "worktree path should exist after creation");
        assert!(branch_exists(dir.path(), "wt-keep"), "branch should exist after worktree creation");

        // keep sequence: remove worktree only, do NOT delete branch
        remove_worktree(dir.path(), &worktree_path, true)
            .await
            .expect("remove_worktree should succeed");

        assert!(!worktree_path.exists(), "worktree path should be gone");
        assert!(branch_exists(dir.path(), "wt-keep"), "branch should still exist in keep mode");
    }

    // ── working_tree_dirty / commits_ahead_of_base tests ────────────────────────

    #[tokio::test]
    async fn working_tree_dirty_detects_untracked_and_clean() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let ps = p.to_str().unwrap();
        run_git(&["init", "-q", ps]).await.unwrap();
        run_git(&["-C", ps, "config", "user.email", "t@t"]).await.unwrap();
        run_git(&["-C", ps, "config", "user.name", "t"]).await.unwrap();
        std::fs::write(p.join("a.txt"), "x").unwrap();
        run_git(&["-C", ps, "add", "."]).await.unwrap();
        run_git(&["-C", ps, "commit", "-qm", "init"]).await.unwrap();

        // Clean tree.
        assert!(!working_tree_dirty(p).await.unwrap());

        // Untracked file → dirty.
        std::fs::write(p.join("b.txt"), "y").unwrap();
        assert!(working_tree_dirty(p).await.unwrap());
    }

    #[tokio::test]
    async fn commits_ahead_of_base_true_when_branch_has_extra_commits() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        let ps = p.to_str().unwrap();
        run_git(&["init", "-q", "-b", "main", ps]).await.unwrap();
        run_git(&["-C", ps, "config", "user.email", "t@t"]).await.unwrap();
        run_git(&["-C", ps, "config", "user.name", "t"]).await.unwrap();
        std::fs::write(p.join("a.txt"), "x").unwrap();
        run_git(&["-C", ps, "add", "."]).await.unwrap();
        run_git(&["-C", ps, "commit", "-qm", "base"]).await.unwrap();

        // On main == base: nothing ahead.
        assert!(!commits_ahead_of_base(p, "main").await.unwrap());

        // Create a branch with an extra commit → ahead of main.
        run_git(&["-C", ps, "checkout", "-q", "-b", "feature"]).await.unwrap();
        std::fs::write(p.join("c.txt"), "z").unwrap();
        run_git(&["-C", ps, "add", "."]).await.unwrap();
        run_git(&["-C", ps, "commit", "-qm", "feature work"]).await.unwrap();
        assert!(commits_ahead_of_base(p, "main").await.unwrap());
    }

    // ── ref_exists / fetch refspec tests ────────────────────────────────────────

    #[tokio::test]
    async fn ref_exists_true_for_present_ref_false_for_missing() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path();
        let run = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(p).output().unwrap()
        };
        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);
        std::fs::write(p.join("a.txt"), "a").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "c0"]);

        assert!(ref_exists(p, "HEAD").await, "HEAD must resolve");
        assert!(ref_exists(p, "main").await, "main must resolve");
        assert!(!ref_exists(p, "origin/main").await, "no remote-tracking ref yet");
        assert!(!ref_exists(p, "refs/remotes/origin/main").await, "explicit missing ref");
    }

    #[tokio::test]
    async fn fetch_updates_remote_tracking_ref_via_refspec() {
        let tmp = tempfile::TempDir::new().unwrap();
        let bare = tmp.path().join("origin.git");
        let work = tmp.path().join("work");
        let run_in = |dir: &std::path::Path, args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(dir).output().unwrap()
        };

        // Bare "remote" with one commit on main.
        std::fs::create_dir_all(&bare).unwrap();
        run_in(tmp.path(), &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()]);
        let seed = tmp.path().join("seed");
        std::fs::create_dir_all(&seed).unwrap();
        run_in(&seed, &["init", "-q", "-b", "main"]);
        run_in(&seed, &["config", "user.email", "t@t"]);
        run_in(&seed, &["config", "user.name", "t"]);
        std::fs::write(seed.join("a.txt"), "a").unwrap();
        run_in(&seed, &["add", "."]);
        run_in(&seed, &["commit", "-qm", "c0"]);
        run_in(&seed, &["remote", "add", "origin", bare.to_str().unwrap()]);
        run_in(&seed, &["push", "-q", "origin", "main"]);

        // Fresh clone-like work repo with origin configured but no tracking ref fetched.
        run_in(tmp.path(), &["init", "-q", "-b", "main", work.to_str().unwrap()]);
        run_in(&work, &["config", "user.email", "t@t"]);
        run_in(&work, &["config", "user.name", "t"]);
        run_in(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        assert!(!ref_exists(&work, "origin/main").await, "tracking ref absent before fetch");

        fetch(&work, "origin", "main").await.expect("fetch should succeed");
        assert!(ref_exists(&work, "origin/main").await, "tracking ref present after refspec fetch");
    }

    #[tokio::test]
    async fn diff_and_ahead_exclude_merged_in_origin_commits() {
        // Scenario (TASK-144): task branch forks from origin/main@C0, upstream advances
        // origin/main to C1, the branch does its own work (C2) then merges origin/main
        // (C1) in to resolve conflicts. Comparing against a stale base (C0) would show
        // the merged-in upstream file as "ours"; comparing against fresh origin/main
        // (C1) must show only the task's own file.
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path();
        let run = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(p)
                .output()
                .unwrap();
            assert!(out.status.success(), "git {:?} failed: {}", args, String::from_utf8_lossy(&out.stderr));
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };

        run(&["init", "-q", "-b", "main"]);
        run(&["config", "user.email", "t@t"]);
        run(&["config", "user.name", "t"]);

        // C0: the fork point.
        std::fs::write(p.join("base.txt"), "base").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "c0"]);
        let c0 = run(&["rev-parse", "HEAD"]);
        run(&["update-ref", "refs/remotes/origin/main", &c0]);

        // Task branch off C0.
        run(&["checkout", "-q", "-b", "feat"]);

        // Upstream advances origin/main to C1 on a side branch, then point the
        // tracking ref at it (simulates someone else merging to main).
        run(&["checkout", "-q", "-b", "upstream-tmp", &c0]);
        std::fs::write(p.join("upstream.txt"), "upstream").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "c1 upstream"]);
        let c1 = run(&["rev-parse", "HEAD"]);
        run(&["update-ref", "refs/remotes/origin/main", &c1]);

        // Back on feat: task's own work (C2), then merge origin/main (C1) in.
        run(&["checkout", "-q", "feat"]);
        std::fs::write(p.join("feat.txt"), "feat work").unwrap();
        run(&["add", "."]);
        run(&["commit", "-qm", "c2 feat"]);
        run(&["merge", "-q", "--no-edit", "origin/main"]);

        // Fresh base (origin/main = C1): only the task's own file.
        let files_fresh = changed_files(p, "origin/main").await.unwrap();
        let paths_fresh: Vec<&str> = files_fresh.iter().map(|f| f.path.as_str()).collect();
        assert!(paths_fresh.contains(&"feat.txt"), "fresh base must include task's own file; got {paths_fresh:?}");
        assert!(!paths_fresh.contains(&"upstream.txt"), "fresh base must exclude merged-in upstream file; got {paths_fresh:?}");

        let diff_fresh = diff_against_base(p, "origin/main").await.unwrap();
        assert!(diff_fresh.contains("feat.txt"), "fresh diff must mention feat.txt");
        assert!(!diff_fresh.contains("upstream.txt"), "fresh diff must not mention upstream.txt");

        // Contrast: the stale base (C0) wrongly includes the merged-in upstream file —
        // this is exactly the bug TASK-144 fixes by resolving to origin/main.
        let files_stale = changed_files(p, &c0).await.unwrap();
        let paths_stale: Vec<&str> = files_stale.iter().map(|f| f.path.as_str()).collect();
        assert!(paths_stale.contains(&"upstream.txt"), "stale base reproduces the bug (upstream shown as ours); got {paths_stale:?}");

        // Teardown gate: still "ahead" of fresh base because of the task's own C2.
        assert!(commits_ahead_of_base(p, "origin/main").await.unwrap(), "task has its own commit ahead of origin/main");
    }
}
