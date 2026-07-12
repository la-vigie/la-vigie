//! Worktree adoption detection (TASK-125).
//!
//! When a task's derived worktree path already exists on disk, `git worktree add`
//! fails. Instead of surfacing a cryptic git error, we classify the target path
//! into one of three states and let the caller decide:
//!   * `Vacant`  — nothing there → create the worktree normally.
//!   * `Adopt`   — a registered worktree on the intended branch is already present
//!                 → reuse it (skip `worktree add`).
//!   * `Conflict`— the path is occupied by something that does NOT match → warn,
//!                 don't clobber.
//!
//! This file is the PURE core: parsing `git worktree list --porcelain`, matching a
//! target path against the parsed entries, and classifying the state. It is fully
//! unit-tested here. The thin async wrapper that runs the git command and
//! canonicalizes filesystem paths lives in `git::worktree_state` (mod.rs).

use std::path::{Path, PathBuf};

/// How a registered worktree has its HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registration {
    /// Checked out on a local branch (short name, e.g. `task-125`).
    OnBranch(String),
    /// Detached HEAD (no branch).
    Detached,
    /// A bare worktree entry (the main bare repo).
    Bare,
}

/// One entry from `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub registration: Registration,
}

/// The decision for a task's target worktree path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeAdoption {
    /// Nothing at the path — create the worktree normally.
    Vacant,
    /// A registered worktree on the intended branch with its dir present — adopt it.
    Adopt,
    /// A leftover/orphaned or stale-registered worktree occupies the path but is
    /// not usable (its git metadata is gone / its directory is gone). Clean it up
    /// and recreate — this is what unblocks a task name that a broken worktree
    /// would otherwise burn forever. The existing branch (if any) is preserved.
    Reclaim { reason: String },
    /// The path is occupied by something that doesn't match — warn, don't clobber.
    Conflict { reason: String },
}

/// Strip a `refs/heads/` prefix to get a short branch name; leave anything else
/// (already-short names, other ref namespaces) as-is.
fn short_branch(refname: &str) -> String {
    refname
        .strip_prefix("refs/heads/")
        .unwrap_or(refname)
        .to_string()
}

/// Parse `git worktree list --porcelain` into entries. Blocks are separated by
/// blank lines; each starts with `worktree <path>` followed by attribute lines
/// (`HEAD <sha>`, `branch refs/heads/<name>`, `detached`, `bare`, `locked`, …).
/// Unknown attribute lines are ignored.
pub fn parse_worktree_list(porcelain: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut path: Option<PathBuf> = None;
    let mut branch: Option<String> = None;
    let mut bare = false;

    fn flush(
        entries: &mut Vec<WorktreeEntry>,
        path: &mut Option<PathBuf>,
        branch: &mut Option<String>,
        bare: &mut bool,
    ) {
        if let Some(p) = path.take() {
            let registration = if *bare {
                Registration::Bare
            } else if let Some(b) = branch.take() {
                Registration::OnBranch(b)
            } else {
                // A non-bare worktree with no `branch` line is detached (git emits
                // an explicit `detached` line for these).
                Registration::Detached
            };
            entries.push(WorktreeEntry { path: p, registration });
        }
        *branch = None;
        *bare = false;
    }

    for line in porcelain.lines() {
        let line = line.trim_end();
        if let Some(rest) = line.strip_prefix("worktree ") {
            // New block: flush the previous entry first.
            flush(&mut entries, &mut path, &mut branch, &mut bare);
            path = Some(PathBuf::from(rest));
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = Some(short_branch(rest));
        } else if line == "bare" {
            bare = true;
        }
        // Everything else (HEAD, detached, locked, prunable, blank lines) is
        // ignored; the next `worktree ` line (or EOF) flushes the current block.
    }
    flush(&mut entries, &mut path, &mut branch, &mut bare);
    entries
}

/// Find the registration of the entry whose path equals `target` (exact
/// `PathBuf` equality — callers canonicalize both sides beforehand so macOS
/// `/var` vs `/private/var` doesn't cause a spurious miss). `None` = the target
/// path is not a registered worktree.
pub fn match_registration(entries: &[WorktreeEntry], target: &Path) -> Option<Registration> {
    entries
        .iter()
        .find(|e| e.path == target)
        .map(|e| e.registration.clone())
}

/// Classify a target worktree path into an adoption decision. Pure: all
/// filesystem/git facts are passed in.
///
/// * `registration`        — the target's registration, or `None` if unregistered.
/// * `path_exists`         — whether the target path exists on disk.
/// * `is_orphaned_worktree`— the path is a leftover worktree checkout (has a
///   `gitdir:` `.git` pointer) that git no longer tracks — reclaimable, not
///   foreign user data. Only meaningful when `path_exists && registration.is_none()`.
/// * `already_task_owned`  — whether a La Vigie task row already points here.
/// * `intended_branch`     — the branch the new task wants (short name).
/// * `target_path`         — for human-readable messages.
pub fn classify_worktree_target(
    registration: Option<&Registration>,
    path_exists: bool,
    is_orphaned_worktree: bool,
    already_task_owned: bool,
    intended_branch: &str,
    target_path: &str,
) -> WorktreeAdoption {
    // A live task already owns this worktree: adopting would create a duplicate
    // task on the same path (a session-lifecycle hazard). Never silently reuse.
    if already_task_owned {
        return WorktreeAdoption::Conflict {
            reason: format!("another task already uses the worktree at {target_path}"),
        };
    }

    match registration {
        None => {
            if !path_exists {
                WorktreeAdoption::Vacant
            } else if is_orphaned_worktree {
                // A leftover worktree directory git no longer tracks (deregistered
                // / broken admin data). Reclaim it so the task name isn't burned.
                WorktreeAdoption::Reclaim {
                    reason: format!(
                        "a leftover worktree already exists at {target_path} — it will be cleaned up and recreated"
                    ),
                }
            } else {
                WorktreeAdoption::Conflict {
                    reason: format!(
                        "a directory already exists at {target_path} but is not a git worktree — remove it or choose a different name"
                    ),
                }
            }
        }
        Some(Registration::OnBranch(b)) if b == intended_branch => {
            if path_exists {
                WorktreeAdoption::Adopt
            } else {
                // Registered but its directory is gone: a stale admin entry that
                // blocks the name. Prune + recreate.
                WorktreeAdoption::Reclaim {
                    reason: format!(
                        "a stale worktree entry for {b} exists but its directory is gone at {target_path} — it will be pruned and recreated"
                    ),
                }
            }
        }
        Some(Registration::OnBranch(b)) => WorktreeAdoption::Conflict {
            reason: format!(
                "a worktree at {target_path} is checked out on branch {b}, not {intended_branch}"
            ),
        },
        Some(Registration::Detached) => WorktreeAdoption::Conflict {
            reason: format!("a worktree at {target_path} has a detached HEAD"),
        },
        Some(Registration::Bare) => WorktreeAdoption::Conflict {
            reason: format!("{target_path} is a bare worktree"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
worktree /repo/main
HEAD aaaa
branch refs/heads/main

worktree /wt/task-125
HEAD bbbb
branch refs/heads/task-125

worktree /wt/detached
HEAD cccc
detached

worktree /repo/bare
bare
";

    #[test]
    fn parse_reads_all_blocks_with_short_branches() {
        let entries = parse_worktree_list(SAMPLE);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].path, PathBuf::from("/repo/main"));
        assert_eq!(entries[0].registration, Registration::OnBranch("main".into()));
        assert_eq!(entries[1].registration, Registration::OnBranch("task-125".into()));
        assert_eq!(entries[2].registration, Registration::Detached);
        assert_eq!(entries[3].registration, Registration::Bare);
    }

    #[test]
    fn parse_empty_is_empty() {
        assert!(parse_worktree_list("").is_empty());
        assert!(parse_worktree_list("\n\n").is_empty());
    }

    #[test]
    fn parse_tolerates_no_trailing_blank_line() {
        let entries = parse_worktree_list("worktree /a\nHEAD x\nbranch refs/heads/b");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].registration, Registration::OnBranch("b".into()));
    }

    #[test]
    fn match_registration_hits_and_misses() {
        let entries = parse_worktree_list(SAMPLE);
        assert_eq!(
            match_registration(&entries, Path::new("/wt/task-125")),
            Some(Registration::OnBranch("task-125".into()))
        );
        assert_eq!(match_registration(&entries, Path::new("/wt/nope")), None);
    }

    #[test]
    fn classify_vacant_when_nothing_there() {
        let d = classify_worktree_target(None, false, false, false, "task-125", "/wt/task-125");
        assert_eq!(d, WorktreeAdoption::Vacant);
    }

    #[test]
    fn classify_adopt_when_registered_on_branch_and_present() {
        let reg = Registration::OnBranch("task-125".into());
        let d = classify_worktree_target(Some(&reg), true, false, false, "task-125", "/wt/task-125");
        assert_eq!(d, WorktreeAdoption::Adopt);
    }

    #[test]
    fn classify_reclaim_orphaned_leftover_worktree() {
        // Not registered, dir present, and it's a leftover worktree checkout.
        let d = classify_worktree_target(None, true, true, false, "task-125", "/wt/task-125");
        match d {
            WorktreeAdoption::Reclaim { reason } => assert!(reason.contains("leftover worktree")),
            other => panic!("expected Reclaim, got {other:?}"),
        }
    }

    #[test]
    fn classify_conflict_bare_dir_not_a_worktree() {
        // Present but NOT a leftover worktree (no gitdir pointer) → don't clobber.
        let d = classify_worktree_target(None, true, false, false, "task-125", "/wt/task-125");
        match d {
            WorktreeAdoption::Conflict { reason } => assert!(reason.contains("not a git worktree")),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn classify_reclaim_registered_but_dir_missing() {
        // Stale admin entry (registered, dir gone) burns the name → reclaim.
        let reg = Registration::OnBranch("task-125".into());
        let d = classify_worktree_target(Some(&reg), false, false, false, "task-125", "/wt/task-125");
        match d {
            WorktreeAdoption::Reclaim { reason } => assert!(reason.contains("pruned")),
            other => panic!("expected Reclaim, got {other:?}"),
        }
    }

    #[test]
    fn classify_conflict_wrong_branch() {
        let reg = Registration::OnBranch("other".into());
        let d = classify_worktree_target(Some(&reg), true, false, false, "task-125", "/wt/task-125");
        match d {
            WorktreeAdoption::Conflict { reason } => {
                assert!(reason.contains("branch other"));
                assert!(reason.contains("not task-125"));
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn classify_conflict_detached_and_bare() {
        let det = classify_worktree_target(
            Some(&Registration::Detached), true, false, false, "task-125", "/wt/x",
        );
        assert!(matches!(det, WorktreeAdoption::Conflict { .. }));
        let bare = classify_worktree_target(
            Some(&Registration::Bare), true, false, false, "task-125", "/wt/x",
        );
        assert!(matches!(bare, WorktreeAdoption::Conflict { .. }));
    }

    #[test]
    fn classify_conflict_when_already_task_owned_even_if_matching() {
        // Owned takes priority over an otherwise-adoptable match.
        let reg = Registration::OnBranch("task-125".into());
        let d = classify_worktree_target(Some(&reg), true, false, true, "task-125", "/wt/task-125");
        match d {
            WorktreeAdoption::Conflict { reason } => assert!(reason.contains("another task")),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }
}
