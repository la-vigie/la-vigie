//! GitHub layer: PR operations via the `gh` CLI (plus `git push` for create).
//!
//! Scope: this module is the GitHub layer only. No Tauri commands here — those
//! live in `crate::commands`, which calls this module's public functions.
//!
//! The pure JSON parsers ([`parse_pr_status`], [`parse_comments`]) are
//! unit-tested against fixtures captured from real `gh` output (see the
//! `fixtures/` dir + the T0 verify-claims report). The thin async wrappers that
//! actually invoke `gh`/`git` are not unit-tested (they need `gh` + network),
//! consistent with the `git` layer.

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde_json::Value;
use tokio::process::Command;

// ── Types (serialized to the frontend over IPC) ───────────────────────────────

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatus {
    pub number: i64,
    pub url: String,
    pub title: String,
    /// OPEN | MERGED | CLOSED
    pub state: String,
    pub is_draft: bool,
    /// MERGEABLE | CONFLICTING | UNKNOWN
    pub mergeable: String,
    /// APPROVED | CHANGES_REQUESTED | REVIEW_REQUIRED — `None` when gh returns "".
    pub review_decision: Option<String>,
    pub checks: Vec<PrCheck>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrCheck {
    pub name: String,
    /// Normalized: "success" | "failure" | "pending" | "neutral".
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrComment {
    pub author: String,
    pub body: String,
    pub created_at: String,
    pub path: Option<String>,
    pub line: Option<i64>,
    /// "issue_comment" | "review" | "inline"
    pub kind: String,
    /// Review state for kind == "review" (APPROVED|CHANGES_REQUESTED|COMMENTED|
    /// DISMISSED|PENDING); `None` otherwise.
    pub state: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhStatus {
    pub available: bool,
    pub authenticated: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrRef {
    pub number: i64,
    pub url: String,
}

// ── Pure parsers (unit-tested against fixtures) ───────────────────────────────

/// Parse the JSON object from
/// `gh pr view <branch> --json number,url,title,state,isDraft,mergeable,reviewDecision,statusCheckRollup`.
pub fn parse_pr_status(json: &str) -> Result<PrStatus> {
    let v: Value = serde_json::from_str(json).context("parsing gh pr view json")?;

    let number = v["number"]
        .as_i64()
        .ok_or_else(|| anyhow!("gh pr view json missing `number`"))?;

    let review_decision = match v["reviewDecision"].as_str() {
        Some("") | None => None,
        Some(s) => Some(s.to_string()),
    };

    let checks = v["statusCheckRollup"]
        .as_array()
        .map(|arr| arr.iter().filter_map(normalize_check).collect())
        .unwrap_or_default();

    Ok(PrStatus {
        number,
        url: v["url"].as_str().unwrap_or_default().to_string(),
        title: v["title"].as_str().unwrap_or_default().to_string(),
        state: v["state"].as_str().unwrap_or_default().to_string(),
        is_draft: v["isDraft"].as_bool().unwrap_or(false),
        mergeable: v["mergeable"].as_str().unwrap_or("UNKNOWN").to_string(),
        review_decision,
        checks,
    })
}

/// Map a single `statusCheckRollup` element to a normalized `PrCheck`.
/// Elements come in two shapes discriminated by `__typename` (see T0 report):
/// - `CheckRun`: name from `name`, result from `conclusion` (empty while running).
/// - `StatusContext`: name from `context`, result from `state`.
fn normalize_check(elem: &Value) -> Option<PrCheck> {
    match elem["__typename"].as_str() {
        Some("CheckRun") => {
            let name = elem["name"].as_str()?.to_string();
            let status = if elem["status"].as_str() != Some("COMPLETED") {
                "pending"
            } else {
                match elem["conclusion"].as_str().unwrap_or("") {
                    "SUCCESS" => "success",
                    "FAILURE" | "TIMED_OUT" | "STARTUP_FAILURE" | "ACTION_REQUIRED" => "failure",
                    "NEUTRAL" | "SKIPPED" | "STALE" | "CANCELLED" => "neutral",
                    "" => "pending",
                    _ => "neutral",
                }
            }
            .to_string();
            Some(PrCheck { name, status })
        }
        Some("StatusContext") => {
            let name = elem["context"].as_str()?.to_string();
            let status = match elem["state"].as_str().unwrap_or("") {
                "SUCCESS" => "success",
                "FAILURE" | "ERROR" => "failure",
                "PENDING" | "EXPECTED" => "pending",
                _ => "neutral",
            }
            .to_string();
            Some(PrCheck { name, status })
        }
        _ => None,
    }
}

/// Merge the three comment sources into one chronological list:
/// - `view_json`'s `comments[]` (issue-level) and `reviews[]` (with state),
/// - `inline_json` (a REST array from `gh api .../pulls/<n>/comments`, inline).
pub fn parse_comments(view_json: &str, inline_json: &str) -> Result<Vec<PrComment>> {
    let mut out: Vec<PrComment> = Vec::new();

    let view: Value = serde_json::from_str(view_json).context("parsing gh pr view comments json")?;

    if let Some(arr) = view["comments"].as_array() {
        for c in arr {
            out.push(PrComment {
                author: c["author"]["login"].as_str().unwrap_or_default().to_string(),
                body: c["body"].as_str().unwrap_or_default().to_string(),
                created_at: c["createdAt"].as_str().unwrap_or_default().to_string(),
                path: None,
                line: None,
                kind: "issue_comment".to_string(),
                state: None,
            });
        }
    }

    if let Some(arr) = view["reviews"].as_array() {
        for r in arr {
            out.push(PrComment {
                author: r["author"]["login"].as_str().unwrap_or_default().to_string(),
                body: r["body"].as_str().unwrap_or_default().to_string(),
                created_at: r["submittedAt"].as_str().unwrap_or_default().to_string(),
                path: None,
                line: None,
                kind: "review".to_string(),
                state: r["state"].as_str().map(|s| s.to_string()),
            });
        }
    }

    let inline: Value =
        serde_json::from_str(inline_json).context("parsing inline review comments json")?;
    if let Some(arr) = inline.as_array() {
        for c in arr {
            // `line` is null when the commented line no longer exists; fall back
            // to `original_line`.
            let line = c["line"].as_i64().or_else(|| c["original_line"].as_i64());
            out.push(PrComment {
                author: c["user"]["login"].as_str().unwrap_or_default().to_string(),
                body: c["body"].as_str().unwrap_or_default().to_string(),
                created_at: c["created_at"].as_str().unwrap_or_default().to_string(),
                path: c["path"].as_str().map(|s| s.to_string()),
                line,
                kind: "inline".to_string(),
                state: None,
            });
        }
    }

    // ISO-8601 strings sort chronologically lexicographically.
    out.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    Ok(out)
}

// ── Thin async wrappers (invoke gh/git; not unit-tested) ──────────────────────

/// Run `gh` in `cwd`, returning trimmed stdout on success; Err includes stderr.
async fn run_gh(cwd: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("gh")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .with_context(|| format!("failed to spawn gh {:?}", args))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "gh {:?} failed (status {:?}): {}",
            args,
            output.status.code(),
            stderr.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Whether `gh` is available on PATH and the user is authenticated.
pub async fn gh_status() -> GhStatus {
    let available = Command::new("gh")
        .arg("--version")
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);

    let authenticated = if available {
        Command::new("gh")
            .args(["auth", "status"])
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    } else {
        false
    };

    GhStatus {
        available,
        authenticated,
    }
}

/// PR status for `branch`, or `None` when no PR exists for it.
pub async fn pr_status(worktree: &Path, branch: &str) -> Result<Option<PrStatus>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            branch,
            "--json",
            "number,url,title,state,isDraft,mergeable,reviewDecision,statusCheckRollup",
        ])
        .current_dir(worktree)
        .output()
        .await
        .context("failed to spawn gh pr view")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let lower = stderr.to_lowercase();
        if lower.contains("no pull requests found")
            || lower.contains("no open pull requests")
            || lower.contains("no pull request")
        {
            return Ok(None);
        }
        return Err(anyhow!(
            "gh pr view failed (status {:?}): {}",
            output.status.code(),
            stderr.trim()
        ));
    }

    let json = String::from_utf8_lossy(&output.stdout);
    Ok(Some(parse_pr_status(&json)?))
}

/// All comments/reviews for PR `number` (issue + review + inline), chronological.
pub async fn pr_comments(worktree: &Path, number: i64) -> Result<Vec<PrComment>> {
    let number_str = number.to_string();
    let view_json = run_gh(worktree, &["pr", "view", &number_str, "--json", "comments,reviews"]).await?;

    // Resolve owner/name to call the REST inline-comments endpoint.
    let repo_json = run_gh(worktree, &["repo", "view", "--json", "owner,name"]).await?;
    let rv: Value = serde_json::from_str(&repo_json).context("parsing gh repo view json")?;
    let owner = rv["owner"]["login"]
        .as_str()
        .ok_or_else(|| anyhow!("gh repo view missing owner.login"))?;
    let name = rv["name"]
        .as_str()
        .ok_or_else(|| anyhow!("gh repo view missing name"))?;
    let endpoint = format!("repos/{owner}/{name}/pulls/{number}/comments");

    // Inline comments are best-effort: a failure here shouldn't drop the
    // top-level comments/reviews we already have.
    let inline_json = run_gh(worktree, &["api", &endpoint])
        .await
        .unwrap_or_else(|_| "[]".to_string());

    parse_comments(&view_json, &inline_json)
}

/// Push `branch` to origin, open a PR, and return its number/url.
pub async fn create_pr(
    worktree: &Path,
    branch: &str,
    base: &str,
    title: &str,
    body: &str,
    draft: bool,
) -> Result<PrRef> {
    // Push the branch first (PR creation needs it on origin).
    let push = Command::new("git")
        .args(["push", "-u", "origin", branch])
        .current_dir(worktree)
        .output()
        .await
        .context("failed to spawn git push")?;
    if !push.status.success() {
        let stderr = String::from_utf8_lossy(&push.stderr);
        return Err(anyhow!(
            "git push failed (status {:?}): {}",
            push.status.code(),
            stderr.trim()
        ));
    }

    let mut args: Vec<&str> = vec![
        "pr", "create", "-H", branch, "-B", base, "-t", title, "-b", body,
    ];
    if draft {
        args.push("-d");
    }
    run_gh(worktree, &args).await?;

    // Re-query to get the structured number/url.
    let json = run_gh(worktree, &["pr", "view", branch, "--json", "number,url"]).await?;
    let v: Value = serde_json::from_str(&json).context("parsing gh pr view (number,url)")?;
    Ok(PrRef {
        number: v["number"]
            .as_i64()
            .ok_or_else(|| anyhow!("gh pr view missing number after create"))?,
        url: v["url"].as_str().unwrap_or_default().to_string(),
    })
}

/// Squash-merge PR `number`. Not forced — `gh` errors if it isn't mergeable.
pub async fn merge_pr(worktree: &Path, number: i64) -> Result<()> {
    let number_str = number.to_string();
    run_gh(worktree, &["pr", "merge", &number_str, "--squash"]).await?;
    Ok(())
}

// ── Tests (pure parsers only) ─────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const PR_VIEW: &str = include_str!("fixtures/pr_view.json");
    const PR_COMMENTS: &str = include_str!("fixtures/pr_comments.json");
    const PR_INLINE: &str = include_str!("fixtures/pr_inline_comments.json");

    /// The fixture wraps real `gh pr view` objects in an `examples[]` array;
    /// return one example as a JSON string shaped like real gh output.
    fn pr_view_example(idx: usize) -> String {
        let v: Value = serde_json::from_str(PR_VIEW).unwrap();
        v["examples"][idx].to_string()
    }

    #[test]
    fn parse_pr_status_checkrun_fields_and_normalization() {
        // examples[0]: open draft PR with CheckRun checks.
        let s = parse_pr_status(&pr_view_example(0)).unwrap();
        assert_eq!(s.number, 13707);
        assert_eq!(s.state, "OPEN");
        assert!(s.is_draft);
        assert_eq!(s.mergeable, "MERGEABLE");
        assert_eq!(s.review_decision.as_deref(), Some("REVIEW_REQUIRED"));

        let checks: Vec<(&str, &str)> = s
            .checks
            .iter()
            .map(|c| (c.name.as_str(), c.status.as_str()))
            .collect();
        // SKIPPED conclusion -> neutral; IN_PROGRESS (conclusion "") -> pending.
        assert!(checks.contains(&("label-external", "neutral")));
        assert!(checks.contains(&("test-macOS", "pending")));
    }

    #[test]
    fn parse_pr_status_empty_review_decision_is_none_and_statuscontext() {
        // examples[2]: k8s PR, reviewDecision "" and StatusContext checks.
        let s = parse_pr_status(&pr_view_example(2)).unwrap();
        assert_eq!(s.review_decision, None);

        let checks: Vec<(&str, &str)> = s
            .checks
            .iter()
            .map(|c| (c.name.as_str(), c.status.as_str()))
            .collect();
        // StatusContext: name from `context`, status from `state`.
        assert!(checks.contains(&("pull-kubernetes-gce-master-scale-performance-5000", "pending")));
        assert!(checks.contains(&("EasyCLA", "success")));
    }

    #[test]
    fn parse_pr_status_merged_success_check() {
        // examples[1]: merged PR with a SUCCESS CheckRun.
        let s = parse_pr_status(&pr_view_example(1)).unwrap();
        assert_eq!(s.state, "MERGED");
        assert!(s
            .checks
            .iter()
            .any(|c| c.name == "CodeQL-Build (go)" && c.status == "success"));
    }

    #[test]
    fn parse_comments_merges_issue_review_and_inline() {
        // The inline fixture wraps the REST array in `elements[]`.
        let iv: Value = serde_json::from_str(PR_INLINE).unwrap();
        let inline = iv["elements"].to_string();

        let comments = parse_comments(PR_COMMENTS, &inline).unwrap();

        assert!(comments
            .iter()
            .any(|c| c.kind == "issue_comment" && c.author == "kubernetes-prow"));
        assert!(comments.iter().any(|c| c.kind == "review"
            && c.author == "Cooper-Runstein"
            && c.state.as_deref() == Some("COMMENTED")));
        // Inline with a real line.
        assert!(comments.iter().any(|c| c.kind == "inline"
            && c.path.as_deref() == Some("pkg/cmd/pr/checks/output_test.go")
            && c.line == Some(15)));
        // Inline whose `line` is null falls back to original_line (55).
        assert!(comments
            .iter()
            .any(|c| c.kind == "inline" && c.line == Some(55)));
    }
}
