//! Runs a repo's setup step in a freshly-created worktree: either a per-repo
//! command (via the user's interactive shell) or the committed `.vigie/setup.sh`.
//! Streams combined stdout/stderr through a caller-supplied `emit` sink so the
//! command layer can forward them over a Tauri Channel while this stays testable.

use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Streamed to the frontend over the `Channel` while a task's setup runs.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SetupEvent {
    /// A line of combined stdout/stderr (newline included).
    Output { data: String },
    /// No `.vigie/setup.sh` existed; nothing ran.
    Skipped,
    /// The setup script finished with this status code.
    Exit { code: i32 },
}

/// Result of attempting setup for a worktree.
pub enum SetupOutcome {
    Skipped,
    Ran { code: i32 },
}

fn shell() -> &'static str {
    if Path::new("/bin/bash").exists() {
        "/bin/bash"
    } else {
        "/bin/sh"
    }
}

/// Whether `run_setup` would actually run something (vs. skip): true if a
/// non-empty `setup_command` is set, otherwise true iff `.vigie/setup.sh` exists.
pub fn will_run(worktree_path: &Path, setup_command: Option<&str>) -> bool {
    match setup_command {
        Some(cmd) if !cmd.trim().is_empty() => true,
        _ => worktree_path.join(".vigie").join("setup.sh").exists(),
    }
}

/// Run a repo's setup step in `worktree_path` (cwd = worktree), streaming output
/// via `emit`. If `setup_command` is set, run it through the user's interactive
/// shell (`$SHELL -ic`) so shell functions/aliases resolve. Otherwise fall back
/// to `<worktree>/.vigie/setup.sh` if present. Returns `Skipped` when neither
/// applies, `Ran { code }` otherwise. Spawn/IO failures are `Err`.
pub async fn run_setup<F>(
    worktree_path: &Path,
    setup_command: Option<&str>,
    emit: F,
) -> Result<SetupOutcome>
where
    F: Fn(SetupEvent) + Send + Sync + Clone + 'static,
{
    let mut command = match setup_command {
        Some(cmd) if !cmd.trim().is_empty() => {
            // Per-repo command: run through the user's interactive shell so
            // functions/aliases defined in their rc (e.g. a `cwt` function) resolve.
            let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());
            let mut c = Command::new(shell);
            c.arg("-ic").arg(cmd);
            c
        }
        _ => {
            // Fallback: the committed `.vigie/setup.sh` in the worktree.
            let script = worktree_path.join(".vigie").join("setup.sh");
            if !script.exists() {
                emit(SetupEvent::Skipped);
                return Ok(SetupOutcome::Skipped);
            }
            let mut c = Command::new(shell());
            c.arg(&script);
            c
        }
    };

    let mut child = command
        .current_dir(worktree_path)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn setup")?;

    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");

    let emit_out = emit.clone();
    let out_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            emit_out(SetupEvent::Output { data: format!("{line}\n") });
        }
    });
    let emit_err = emit.clone();
    let err_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            emit_err(SetupEvent::Output { data: format!("{line}\n") });
        }
    });

    let status = child.wait().await.context("waiting for setup script")?;
    let _ = out_task.await;
    let _ = err_task.await;

    let code = status.code().unwrap_or(-1);
    emit(SetupEvent::Exit { code });
    Ok(SetupOutcome::Ran { code })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    fn sink() -> (Arc<Mutex<Vec<SetupEvent>>>, impl Fn(SetupEvent) + Send + Sync + Clone + 'static) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let e = events.clone();
        (events, move |ev| e.lock().unwrap().push(ev))
    }

    fn write_script(dir: &std::path::Path, body: &str) {
        let vigie = dir.join(".vigie");
        std::fs::create_dir_all(&vigie).unwrap();
        std::fs::write(vigie.join("setup.sh"), body).unwrap();
    }

    #[test]
    fn will_run_reflects_command_or_script_presence() {
        let wt = tempdir().unwrap();
        // No command, no script → won't run.
        assert!(!will_run(wt.path(), None));
        assert!(!will_run(wt.path(), Some("   ")));
        // Non-empty command → will run regardless of script.
        assert!(will_run(wt.path(), Some("echo hi")));
        // No command but script present → will run.
        write_script(wt.path(), "echo hi\n");
        assert!(will_run(wt.path(), None));
    }

    #[tokio::test]
    async fn runs_script_in_worktree_cwd_and_streams_output() {
        let wt = tempdir().unwrap();
        write_script(wt.path(), "echo hello\ntouch ran_marker\n");
        let (events, emit) = sink();

        let outcome = run_setup(wt.path(), None, emit).await.unwrap();

        assert!(matches!(outcome, SetupOutcome::Ran { code: 0 }));
        // Ran with cwd = worktree (marker created there).
        assert!(wt.path().join("ran_marker").exists());
        let evs = events.lock().unwrap();
        assert!(evs.iter().any(|e| matches!(e, SetupEvent::Output { data } if data.contains("hello"))));
        assert!(evs.iter().any(|e| matches!(e, SetupEvent::Exit { code: 0 })));
    }

    #[tokio::test]
    async fn non_zero_exit_is_reported() {
        let wt = tempdir().unwrap();
        write_script(wt.path(), "echo boom\nexit 3\n");
        let (events, emit) = sink();

        let outcome = run_setup(wt.path(), None, emit).await.unwrap();

        assert!(matches!(outcome, SetupOutcome::Ran { code: 3 }));
        assert!(events.lock().unwrap().iter().any(|e| matches!(e, SetupEvent::Exit { code: 3 })));
    }

    #[tokio::test]
    async fn missing_script_is_skipped_and_runs_nothing() {
        let wt = tempdir().unwrap();
        let (events, emit) = sink();

        let outcome = run_setup(wt.path(), None, emit).await.unwrap();

        assert!(matches!(outcome, SetupOutcome::Skipped));
        let evs = events.lock().unwrap();
        assert_eq!(*evs, vec![SetupEvent::Skipped]);
    }

    #[tokio::test]
    async fn setup_command_runs_with_worktree_cwd() {
        let wt = tempdir().unwrap();
        let (events, emit) = sink();

        let outcome = run_setup(wt.path(), Some("touch cmd_marker"), emit).await.unwrap();

        // Ran the command with cwd = worktree (marker created there).
        assert!(matches!(outcome, SetupOutcome::Ran { .. }));
        assert!(wt.path().join("cmd_marker").exists());
        assert!(events.lock().unwrap().iter().any(|e| matches!(e, SetupEvent::Exit { .. })));
    }

    #[tokio::test]
    async fn setup_command_takes_precedence_over_vigie_script() {
        let wt = tempdir().unwrap();
        write_script(wt.path(), "touch from_file\n");
        let (_events, emit) = sink();

        run_setup(wt.path(), Some("touch from_command"), emit).await.unwrap();

        assert!(wt.path().join("from_command").exists());
        assert!(!wt.path().join("from_file").exists());
    }

    #[tokio::test]
    async fn empty_command_falls_back_to_vigie_script() {
        let wt = tempdir().unwrap();
        write_script(wt.path(), "touch from_file\n");
        let (_events, emit) = sink();

        let outcome = run_setup(wt.path(), Some("   "), emit).await.unwrap();

        assert!(matches!(outcome, SetupOutcome::Ran { .. }));
        assert!(wt.path().join("from_file").exists());
    }
}
