use std::path::{Path, PathBuf};

use serde::Serialize;

/// A document available for a task, identified by a worktree-relative path that
/// doubles as the read key (`read_doc` validates it against this allow-list).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocRef {
    pub id: String,
    pub label: String,
}

/// The `docs/superpowers/` subdirectories scanned for design/plan docs, paired
/// with the human label shown for each.
const DOC_DIRS: [(&str, &str); 2] = [
    ("docs/superpowers/specs", "Design"),
    ("docs/superpowers/plans", "Plan"),
];

/// Worktree-relative path of a task's own spec file.
pub fn spec_rel_path(ticket: &str) -> String {
    format!("memory/spec_{ticket}.md")
}

/// Resolve the docs available for a task in `worktree`: its `memory/spec_<ticket>.md`
/// (if present) plus design/plan markdown under `docs/superpowers/` that is new on
/// this branch — i.e. not yet on the base branch.
///
/// `branch_docs` is the set of worktree-relative paths that differ from the base
/// branch (added/modified on the branch, or still untracked), computed by the
/// caller via git. Keeping the git query out of this function leaves it pure and
/// unit-testable. The spec lives in the gitignored `memory/` dir, so it never
/// appears in `branch_docs` and is resolved separately from the ticket key.
pub fn resolve_docs(worktree: &Path, ticket: Option<&str>, branch_docs: &[String]) -> Vec<DocRef> {
    let mut docs = Vec::new();

    // 1. The task's own spec (per-worktree, gitignored, may not exist).
    if let Some(ticket) = ticket.filter(|t| !t.is_empty()) {
        // Defense-in-depth: a ticket key is a short id like `AC2-54`, never a
        // path. Reject separators so it can't escape the worktree via the spec
        // path (e.g. `../../foo`).
        if !(ticket.contains('/') || ticket.contains('\\') || ticket.contains("..")) {
            let spec = spec_rel_path(ticket);
            if worktree.join(&spec).is_file() {
                docs.push(DocRef {
                    id: spec,
                    label: format!("Spec ({ticket})"),
                });
            }
        }
    }

    // 2. Design/plan docs that are new on this branch (not on the base branch).
    for (dir, kind) in DOC_DIRS {
        let prefix = format!("{dir}/");
        let mut names: Vec<String> = branch_docs
            .iter()
            // Direct `.md` children of the dir (no nested subdirectories).
            .filter_map(|p| p.strip_prefix(&prefix))
            .filter(|name| name.ends_with(".md") && !name.contains('/'))
            .map(str::to_string)
            .collect();
        names.sort();
        names.dedup();
        for name in names {
            docs.push(DocRef {
                id: format!("{dir}/{name}"),
                label: format!("{kind}: {name}"),
            });
        }
    }

    docs
}

/// Read one task doc by its worktree-relative id, validated against the resolved
/// allow-list (prevents reading arbitrary files / path traversal).
pub fn read_doc(
    worktree: &Path,
    ticket: Option<&str>,
    branch_docs: &[String],
    id: &str,
) -> Result<String, String> {
    if !resolve_docs(worktree, ticket, branch_docs)
        .iter()
        .any(|d| d.id == id)
    {
        return Err(format!("doc not available: {id}"));
    }
    let path: PathBuf = worktree.join(id);
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn spec_rel_path_uses_ticket() {
        assert_eq!(spec_rel_path("AC2-54"), "memory/spec_AC2-54.md");
    }

    #[test]
    fn resolve_returns_empty_without_ticket_or_branch_docs() {
        let dir = tempdir().unwrap();
        assert!(resolve_docs(dir.path(), None, &[]).is_empty());
    }

    #[test]
    fn resolve_rejects_ticket_with_path_separators() {
        let dir = tempdir().unwrap();
        // Even if a file exists at the traversed path, a ticket that looks like
        // a path is refused outright (the spec is the only ticket-derived path).
        fs::create_dir_all(dir.path().join("memory")).unwrap();
        fs::write(dir.path().join("memory/spec_x.md"), "# x").unwrap();
        assert!(resolve_docs(dir.path(), Some("../../etc/passwd"), &[]).is_empty());
        assert!(resolve_docs(dir.path(), Some("a/b"), &[]).is_empty());
    }

    #[test]
    fn resolve_finds_spec_and_branch_design_and_plan() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("memory")).unwrap();
        fs::write(root.join("memory/spec_AC2-54.md"), "# spec").unwrap();

        // Design/plan docs are listed purely because they are new on the branch
        // (passed in `branch_docs`), regardless of ticket-name matching.
        let branch_docs = s(&[
            "docs/superpowers/specs/2026-06-26-ac2-54-foo-design.md",
            "docs/superpowers/plans/2026-06-26-ac2-54-foo.md",
        ]);

        let docs = resolve_docs(root, Some("AC2-54"), &branch_docs);
        let ids: Vec<&str> = docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![
                "memory/spec_AC2-54.md",
                "docs/superpowers/specs/2026-06-26-ac2-54-foo-design.md",
                "docs/superpowers/plans/2026-06-26-ac2-54-foo.md",
            ]
        );
    }

    #[test]
    fn resolve_lists_branch_docs_even_without_ticket_match() {
        let dir = tempdir().unwrap();
        // No ticket at all, but a doc is new on the branch → it still shows.
        let branch_docs = s(&["docs/superpowers/specs/anything-design.md"]);
        let docs = resolve_docs(dir.path(), None, &branch_docs);
        let ids: Vec<&str> = docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["docs/superpowers/specs/anything-design.md"]);
    }

    #[test]
    fn resolve_excludes_docs_not_on_the_branch() {
        let dir = tempdir().unwrap();
        // A doc that exists in the worktree but is *not* in `branch_docs` (i.e.
        // already on the base branch) must be omitted.
        let docs = resolve_docs(dir.path(), Some("AC2-54"), &[]);
        assert!(docs.is_empty());
    }

    #[test]
    fn resolve_ignores_non_md_and_nested_and_unrelated_paths() {
        let dir = tempdir().unwrap();
        let branch_docs = s(&[
            "docs/superpowers/specs/keep-design.md",
            "docs/superpowers/specs/notes.txt",       // not markdown
            "docs/superpowers/specs/sub/nested.md",   // nested, not a direct child
            "src/main.rs",                            // unrelated change
        ]);
        let docs = resolve_docs(dir.path(), None, &branch_docs);
        let ids: Vec<&str> = docs.iter().map(|d| d.id.as_str()).collect();
        assert_eq!(ids, vec!["docs/superpowers/specs/keep-design.md"]);
    }

    #[test]
    fn resolve_omits_missing_spec() {
        let dir = tempdir().unwrap();
        // No memory/ dir at all → no spec entry, no panic.
        let docs = resolve_docs(dir.path(), Some("AC2-99"), &[]);
        assert!(docs.is_empty());
    }

    #[test]
    fn read_doc_rejects_unlisted_id() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("memory")).unwrap();
        fs::write(dir.path().join("memory/spec_AC2-54.md"), "# spec").unwrap();
        // Path traversal / arbitrary read must be refused.
        let err = read_doc(dir.path(), Some("AC2-54"), &[], "../secret.txt").unwrap_err();
        assert!(err.contains("not available"));
    }

    #[test]
    fn read_doc_returns_listed_content() {
        let dir = tempdir().unwrap();
        fs::create_dir_all(dir.path().join("memory")).unwrap();
        fs::write(dir.path().join("memory/spec_AC2-54.md"), "# hello").unwrap();
        let body = read_doc(dir.path(), Some("AC2-54"), &[], "memory/spec_AC2-54.md").unwrap();
        assert_eq!(body, "# hello");
    }
}
