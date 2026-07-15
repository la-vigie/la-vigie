//! Persistence layer for repos and tasks, backed by SQLite (rusqlite).
//!
//! Scope: this module is the storage layer only. No git integration, no Tauri
//! commands, no frontend glue — those belong to later sub-tasks (M1.2/M1.4).

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use crate::agent::spec::AgentSpec;

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repo {
    pub id: String, // uuid string
    pub name: String,
    pub path: String,
    pub default_branch: String,
    pub remote_url: Option<String>,
    pub worktree_root: Option<String>,
    pub setup_command: Option<String>,
    pub default_agent: Option<String>,
    pub auto_start_agent: bool,
    pub initial_prompt: Option<String>,
    pub default_model: Option<String>,
    pub sound_settings: Option<String>,
    pub fetch_remote_base: Option<bool>,
    pub auto_approve: Option<bool>,
    /// TASK-163: default for the New-Task "work in place" checkbox in this repo.
    pub in_place_default: bool,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prompt {
    pub id: String,
    pub label: String,
    pub body: String,
    pub position: i64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String, // uuid string
    pub repo_id: String,
    pub title: String,
    pub worktree_path: String,
    pub branch: String,
    pub base_branch: String,
    pub status: TaskStatus,
    pub created_at: i64, // unix epoch seconds
    pub updated_at: i64,
    pub pr_number: Option<i64>,
    pub pr_url: Option<String>,
    pub ticket_key: Option<String>,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub setup_status: Option<SetupStatus>,
    pub hidden: bool,
    /// The seeded launch prompt for a queued (Pending) task; emitted as the
    /// initial prompt when the task auto-launches on its dependency's merge
    /// (TASK-90). None for normal tasks.
    pub pending_prompt: Option<String>,
    pub auto_approve: Option<bool>,
    /// TASK-163: this task runs in the repo's existing checkout (`worktree_path`
    /// == repo path) instead of an isolated worktree. Gates teardown so the
    /// shared checkout is never `git worktree remove`d.
    pub in_place: bool,
}

/// A repo-scoped recurring schedule: on `cron`, launch a task in `repo_id`
/// whose initial prompt is `prompt` (typically a repo skill like `/security-scan`).
/// TASK-173.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub repo_id: String,
    pub name: String,
    pub prompt: String,
    pub cron: String,
    pub agent: Option<String>,
    pub model: Option<String>,
    pub base_branch: Option<String>,
    /// A one-time (non-recurring) schedule: fires once at `next_run_at`, then
    /// the poller retires it (enabled=false, next_run_at=NULL). One-shot rows
    /// carry an empty `cron` (never cron-parsed). TASK-179.
    pub one_shot: bool,
    /// Skip prepending the repo's `initial_prompt` when this schedule fires
    /// (TASK-181). Defaults to `true` — a scheduled run's prompt is usually
    /// self-contained (e.g. `/security-scan`), so the repo's interactive-onboarding
    /// prompt is noise. Routed into TASK-160's `combineInitialPrompts(null, …)` skip
    /// path via the `task_launched` event.
    pub skip_repo_prompt: bool,
    pub enabled: bool,
    /// Next fire time (unix seconds). `None` ⇒ never fires (e.g. disabled).
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Idle,
    Working,
    NeedsAttention,
    Done,
    Error,
    Pending,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Idle => "idle",
            TaskStatus::Working => "working",
            TaskStatus::NeedsAttention => "needs_attention",
            TaskStatus::Done => "done",
            TaskStatus::Error => "error",
            TaskStatus::Pending => "pending",
        }
    }

    pub fn from_str(s: &str) -> Result<TaskStatus> {
        match s {
            "idle" => Ok(TaskStatus::Idle),
            "working" => Ok(TaskStatus::Working),
            "needs_attention" => Ok(TaskStatus::NeedsAttention),
            "done" => Ok(TaskStatus::Done),
            "error" => Ok(TaskStatus::Error),
            "pending" => Ok(TaskStatus::Pending),
            other => Err(anyhow::anyhow!("invalid TaskStatus: {other}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SetupStatus {
    Running,
    Succeeded,
    Failed,
}

impl SetupStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SetupStatus::Running => "running",
            SetupStatus::Succeeded => "succeeded",
            SetupStatus::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Result<SetupStatus> {
        match s {
            "running" => Ok(SetupStatus::Running),
            "succeeded" => Ok(SetupStatus::Succeeded),
            "failed" => Ok(SetupStatus::Failed),
            other => Err(anyhow::anyhow!("invalid SetupStatus: {other}")),
        }
    }
}

pub struct TaskStore {
    conn: Connection,
}

impl TaskStore {
    /// Open (or create) the DB at `path`, creating tables if absent.
    pub fn open(path: &Path) -> Result<TaskStore> {
        let conn = Connection::open(path).context("opening sqlite database")?;
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS repos (
                id              TEXT PRIMARY KEY,
                name            TEXT NOT NULL,
                path            TEXT NOT NULL,
                default_branch  TEXT NOT NULL,
                remote_url      TEXT,
                worktree_root   TEXT,
                setup_command   TEXT,
                default_agent   TEXT,
                auto_start_agent INTEGER NOT NULL DEFAULT 0,
                initial_prompt  TEXT,
                default_model   TEXT
            );

            CREATE TABLE IF NOT EXISTS tasks (
                id              TEXT PRIMARY KEY,
                repo_id         TEXT NOT NULL,
                title           TEXT NOT NULL,
                worktree_path   TEXT NOT NULL,
                branch          TEXT NOT NULL,
                base_branch     TEXT NOT NULL,
                status          TEXT NOT NULL,
                created_at      INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL,
                pr_number       INTEGER,
                pr_url          TEXT,
                ticket_key      TEXT,
                agent           TEXT,
                model           TEXT,
                setup_status    TEXT,
                pending_prompt  TEXT,
                FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS agents (
                name              TEXT PRIMARY KEY,
                display_name      TEXT NOT NULL,
                binary            TEXT NOT NULL,
                base_args         TEXT NOT NULL,
                resume_args       TEXT NOT NULL,
                extra_args        TEXT NOT NULL,
                prompt_mode       TEXT NOT NULL,
                status            TEXT NOT NULL,
                model_arg         TEXT,
                models_list_args  TEXT
            );

            CREATE TABLE IF NOT EXISTS app_settings (
                key   TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS prompts (
                id       TEXT PRIMARY KEY,
                label    TEXT NOT NULL,
                body     TEXT NOT NULL,
                position INTEGER NOT NULL
            );

            CREATE TABLE IF NOT EXISTS task_dependencies (
                task_id            TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                depends_on_task_id TEXT NOT NULL,
                PRIMARY KEY (task_id, depends_on_task_id)
            );

            CREATE TABLE IF NOT EXISTS schedules (
                id           TEXT PRIMARY KEY,
                repo_id      TEXT NOT NULL REFERENCES repos(id) ON DELETE CASCADE,
                name         TEXT NOT NULL,
                prompt       TEXT NOT NULL,
                cron         TEXT NOT NULL,
                agent        TEXT,
                model        TEXT,
                base_branch  TEXT,
                enabled      INTEGER NOT NULL DEFAULT 1,
                one_shot     INTEGER NOT NULL DEFAULT 0,
                skip_repo_prompt INTEGER NOT NULL DEFAULT 1,
                next_run_at  INTEGER,
                last_run_at  INTEGER,
                created_at   INTEGER NOT NULL,
                updated_at   INTEGER NOT NULL
            );
            ",
        )
        .context("creating schema")?;

        // Migrate DBs created before the PR columns existed: add them if missing.
        let existing: std::collections::HashSet<String> = conn
            .prepare("PRAGMA table_info(tasks)")
            .context("reading tasks table_info")?
            .query_map([], |row| row.get::<_, String>(1))
            .context("querying tasks columns")?
            .collect::<rusqlite::Result<_>>()
            .context("collecting tasks columns")?;
        if !existing.contains("pr_number") {
            conn.execute("ALTER TABLE tasks ADD COLUMN pr_number INTEGER", [])
                .context("adding pr_number column")?;
        }
        if !existing.contains("pr_url") {
            conn.execute("ALTER TABLE tasks ADD COLUMN pr_url TEXT", [])
                .context("adding pr_url column")?;
        }
        if !existing.contains("ticket_key") {
            conn.execute("ALTER TABLE tasks ADD COLUMN ticket_key TEXT", [])
                .context("adding ticket_key column")?;
        }
        if !existing.contains("agent") {
            conn.execute("ALTER TABLE tasks ADD COLUMN agent TEXT", [])
                .context("adding agent column")?;
        }
        if !existing.contains("model") {
            conn.execute("ALTER TABLE tasks ADD COLUMN model TEXT", [])
                .context("adding model column")?;
        }
        if !existing.contains("setup_status") {
            conn.execute("ALTER TABLE tasks ADD COLUMN setup_status TEXT", [])
                .context("adding setup_status column")?;
        }
        if !existing.contains("hidden") {
            conn.execute("ALTER TABLE tasks ADD COLUMN hidden INTEGER NOT NULL DEFAULT 0", [])
                .context("adding hidden column")?;
        }
        if !existing.contains("pending_prompt") {
            conn.execute("ALTER TABLE tasks ADD COLUMN pending_prompt TEXT", [])
                .context("adding pending_prompt column")?;
        }
        if !existing.contains("auto_approve") {
            conn.execute("ALTER TABLE tasks ADD COLUMN auto_approve INTEGER", [])
                .context("adding tasks.auto_approve column")?;
        }
        if !existing.contains("in_place") {
            conn.execute("ALTER TABLE tasks ADD COLUMN in_place INTEGER NOT NULL DEFAULT 0", [])
                .context("adding tasks.in_place column")?;
        }

        // TASK-179: add the one_shot column to schedules tables created before it.
        let sched_cols: std::collections::HashSet<String> = conn
            .prepare("PRAGMA table_info(schedules)")
            .context("reading schedules table_info")?
            .query_map([], |row| row.get::<_, String>(1))
            .context("querying schedules columns")?
            .collect::<rusqlite::Result<_>>()
            .context("collecting schedules columns")?;
        if !sched_cols.contains("one_shot") {
            conn.execute(
                "ALTER TABLE schedules ADD COLUMN one_shot INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .context("adding one_shot column")?;
        }
        // TASK-181: skip-repo-prompt flag on schedules created before it. Default 1
        // (skip) — matches the new-schedule default; a one-time behavior change for
        // pre-existing schedules that relied on the repo prompt being prepended.
        if !sched_cols.contains("skip_repo_prompt") {
            conn.execute(
                "ALTER TABLE schedules ADD COLUMN skip_repo_prompt INTEGER NOT NULL DEFAULT 1",
                [],
            )
            .context("adding skip_repo_prompt column")?;
        }

        // Migrate repos DBs created before the worktree_root column existed.
        let repo_cols: std::collections::HashSet<String> = conn
            .prepare("PRAGMA table_info(repos)")
            .context("reading repos table_info")?
            .query_map([], |row| row.get::<_, String>(1))
            .context("querying repos columns")?
            .collect::<rusqlite::Result<_>>()
            .context("collecting repos columns")?;
        if !repo_cols.contains("worktree_root") {
            conn.execute("ALTER TABLE repos ADD COLUMN worktree_root TEXT", [])
                .context("adding worktree_root column")?;
        }
        if !repo_cols.contains("setup_command") {
            conn.execute("ALTER TABLE repos ADD COLUMN setup_command TEXT", [])
                .context("adding setup_command column")?;
        }
        if !repo_cols.contains("default_agent") {
            conn.execute("ALTER TABLE repos ADD COLUMN default_agent TEXT", [])
                .context("adding default_agent column")?;
        }
        if !repo_cols.contains("auto_start_agent") {
            conn.execute(
                "ALTER TABLE repos ADD COLUMN auto_start_agent INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .context("adding auto_start_agent column")?;
        }
        if !repo_cols.contains("initial_prompt") {
            conn.execute("ALTER TABLE repos ADD COLUMN initial_prompt TEXT", [])
                .context("adding initial_prompt column")?;
        }
        if !repo_cols.contains("default_model") {
            conn.execute("ALTER TABLE repos ADD COLUMN default_model TEXT", [])
                .context("adding default_model column")?;
        }
        if !repo_cols.contains("sound_settings") {
            conn.execute("ALTER TABLE repos ADD COLUMN sound_settings TEXT", [])
                .context("adding sound_settings column")?;
        }
        if !repo_cols.contains("fetch_remote_base") {
            conn.execute("ALTER TABLE repos ADD COLUMN fetch_remote_base INTEGER", [])
                .context("adding fetch_remote_base column")?;
        }
        if !repo_cols.contains("auto_approve") {
            conn.execute("ALTER TABLE repos ADD COLUMN auto_approve INTEGER", [])
                .context("adding repos.auto_approve column")?;
        }
        if !repo_cols.contains("in_place_default") {
            conn.execute(
                "ALTER TABLE repos ADD COLUMN in_place_default INTEGER NOT NULL DEFAULT 0",
                [],
            )
            .context("adding repos.in_place_default column")?;
        }

        // Migrate agents table: add model columns if missing.
        let agent_cols: std::collections::HashSet<String> = conn
            .prepare("PRAGMA table_info(agents)")
            .context("reading agents table_info")?
            .query_map([], |row| row.get::<_, String>(1))
            .context("querying agents columns")?
            .collect::<rusqlite::Result<_>>()
            .context("collecting agents columns")?;
        if !agent_cols.contains("model_arg") {
            conn.execute("ALTER TABLE agents ADD COLUMN model_arg TEXT", [])
                .context("adding agents.model_arg column")?;
        }
        if !agent_cols.contains("models_list_args") {
            conn.execute("ALTER TABLE agents ADD COLUMN models_list_args TEXT", [])
                .context("adding agents.models_list_args column")?;
        }

        Ok(TaskStore { conn })
    }

    // ---- Repos ----

    pub fn insert_repo(&self, repo: &Repo) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO repos (id, name, path, default_branch, remote_url, worktree_root, setup_command, default_agent, auto_start_agent, initial_prompt, default_model, sound_settings, fetch_remote_base, auto_approve, in_place_default)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    repo.id,
                    repo.name,
                    repo.path,
                    repo.default_branch,
                    repo.remote_url,
                    repo.worktree_root,
                    repo.setup_command,
                    repo.default_agent,
                    repo.auto_start_agent,
                    repo.initial_prompt,
                    repo.default_model,
                    repo.sound_settings,
                    repo.fetch_remote_base,
                    repo.auto_approve,
                    repo.in_place_default,
                ],
            )
            .context("inserting repo")?;
        Ok(())
    }

    pub fn get_repo(&self, id: &str) -> Result<Option<Repo>> {
        let repo = self
            .conn
            .query_row(
                "SELECT id, name, path, default_branch, remote_url, worktree_root, setup_command, default_agent, auto_start_agent, initial_prompt, default_model, sound_settings, fetch_remote_base, auto_approve, in_place_default
                 FROM repos WHERE id = ?1",
                params![id],
                Self::row_to_repo,
            )
            .optional()
            .context("getting repo")?;
        Ok(repo)
    }

    pub fn list_repos(&self) -> Result<Vec<Repo>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, name, path, default_branch, remote_url, worktree_root, setup_command, default_agent, auto_start_agent, initial_prompt, default_model, sound_settings, fetch_remote_base, auto_approve, in_place_default FROM repos")
            .context("preparing list_repos")?;
        let rows = stmt
            .query_map([], Self::row_to_repo)
            .context("querying repos")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading repo row")?);
        }
        Ok(out)
    }

    pub fn delete_repo(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM repos WHERE id = ?1", params![id])
            .context("deleting repo")?;
        Ok(())
    }

    pub fn update_repo(
        &self,
        id: &str,
        name: &str,
        default_branch: &str,
        worktree_root: Option<&str>,
        setup_command: Option<&str>,
        auto_start_agent: bool,
        initial_prompt: Option<&str>,
        default_agent: Option<&str>,
    ) -> Result<()> {
        let n = self
            .conn
            .execute(
                "UPDATE repos SET name = ?2, default_branch = ?3, worktree_root = ?4, setup_command = ?5, auto_start_agent = ?6, initial_prompt = ?7, default_agent = ?8 WHERE id = ?1",
                params![id, name, default_branch, worktree_root, setup_command, auto_start_agent, initial_prompt, default_agent],
            )
            .context("updating repo")?;
        if n == 0 {
            anyhow::bail!("repo not found: {id}");
        }
        Ok(())
    }

    pub fn get_app_setting(&self, key: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .context("getting app setting")
    }

    pub fn set_app_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .context("setting app setting")?;
        Ok(())
    }

    pub fn delete_app_setting(&self, key: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM app_settings WHERE key = ?1", params![key])
            .context("deleting app setting")?;
        Ok(())
    }

    /// List every key in `app_settings` (TASK-180: startup orchestrator-marker
    /// prune diffs these against the live repo ids).
    pub fn list_app_setting_keys(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT key FROM app_settings")
            .context("preparing list_app_setting_keys")?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .context("querying app setting keys")?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .context("collecting app setting keys")
    }

    // ---- Prompts ----

    pub fn list_prompts(&self) -> Result<Vec<Prompt>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id, label, body, position FROM prompts ORDER BY position ASC")
            .context("preparing list_prompts")?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Prompt {
                    id: row.get(0)?,
                    label: row.get(1)?,
                    body: row.get(2)?,
                    position: row.get(3)?,
                })
            })
            .context("querying prompts")?;
        rows.collect::<rusqlite::Result<Vec<_>>>().context("collecting prompts")
    }

    pub fn insert_prompt(&self, p: &Prompt) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO prompts (id, label, body, position) VALUES (?1, ?2, ?3, ?4)",
                params![p.id, p.label, p.body, p.position],
            )
            .context("inserting prompt")?;
        Ok(())
    }

    pub fn update_prompt(&self, id: &str, label: &str, body: &str) -> Result<()> {
        let n = self
            .conn
            .execute(
                "UPDATE prompts SET label = ?2, body = ?3 WHERE id = ?1",
                params![id, label, body],
            )
            .context("updating prompt")?;
        if n == 0 {
            anyhow::bail!("prompt not found: {id}");
        }
        Ok(())
    }

    pub fn delete_prompt(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM prompts WHERE id = ?1", params![id])
            .context("deleting prompt")?;
        Ok(())
    }

    pub fn reorder_prompts(&self, ordered_ids: &[String]) -> Result<()> {
        let tx = self.conn.unchecked_transaction().context("beginning reorder transaction")?;
        for (i, id) in ordered_ids.iter().enumerate() {
            tx.execute(
                "UPDATE prompts SET position = ?2 WHERE id = ?1",
                params![id, i as i64],
            )
            .context("reordering prompt")?;
        }
        tx.commit().context("committing reorder")
    }

    pub fn set_repo_sound_settings(&self, repo_id: &str, json: Option<&str>) -> Result<()> {
        let n = self
            .conn
            .execute(
                "UPDATE repos SET sound_settings = ?2 WHERE id = ?1",
                params![repo_id, json],
            )
            .context("updating repo sound_settings")?;
        if n == 0 {
            anyhow::bail!("repo not found: {repo_id}");
        }
        Ok(())
    }

    pub fn set_repo_fetch_remote_base(&self, repo_id: &str, value: Option<bool>) -> Result<()> {
        let n = self
            .conn
            .execute(
                "UPDATE repos SET fetch_remote_base = ?2 WHERE id = ?1",
                params![repo_id, value],
            )
            .context("updating repo fetch_remote_base")?;
        if n == 0 {
            anyhow::bail!("repo not found: {repo_id}");
        }
        Ok(())
    }

    pub fn set_repo_auto_approve(&self, repo_id: &str, value: Option<bool>) -> Result<()> {
        let n = self
            .conn
            .execute(
                "UPDATE repos SET auto_approve = ?2 WHERE id = ?1",
                params![repo_id, value],
            )
            .context("updating repo auto_approve")?;
        if n == 0 {
            anyhow::bail!("repo not found: {repo_id}");
        }
        Ok(())
    }

    pub fn set_repo_in_place_default(&self, repo_id: &str, value: bool) -> Result<()> {
        let n = self
            .conn
            .execute(
                "UPDATE repos SET in_place_default = ?2 WHERE id = ?1",
                params![repo_id, value],
            )
            .context("updating repo in_place_default")?;
        if n == 0 {
            anyhow::bail!("repo not found: {repo_id}");
        }
        Ok(())
    }

    fn row_to_repo(row: &rusqlite::Row) -> rusqlite::Result<Repo> {
        Ok(Repo {
            id: row.get(0)?,
            name: row.get(1)?,
            path: row.get(2)?,
            default_branch: row.get(3)?,
            remote_url: row.get(4)?,
            worktree_root: row.get(5)?,
            setup_command: row.get(6)?,
            default_agent: row.get(7)?,
            auto_start_agent: row.get(8)?,
            initial_prompt: row.get(9)?,
            default_model: row.get(10)?,
            sound_settings: row.get(11)?,
            fetch_remote_base: row.get(12)?,
            auto_approve: row.get(13)?,
            in_place_default: row.get(14)?,
        })
    }

    // ---- Agents (custom definitions) ----

    pub fn upsert_agent(&self, spec: &AgentSpec) -> Result<()> {
        let base = serde_json::to_string(&spec.base_args).context("encoding base_args")?;
        let resume = serde_json::to_string(&spec.resume_args).context("encoding resume_args")?;
        let extra = serde_json::to_string(&spec.extra_args).context("encoding extra_args")?;
        let prompt_mode = serde_json::to_value(spec.prompt_mode)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "none".to_string());
        let status = serde_json::to_value(spec.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "lifecycle".to_string());
        let models_list = spec
            .models_list_args
            .as_ref()
            .map(|v| serde_json::to_string(v).context("encoding models_list_args"))
            .transpose()?;
        self.conn
            .execute(
                "INSERT INTO agents (name, display_name, binary, base_args, resume_args, extra_args, prompt_mode, status, model_arg, models_list_args)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(name) DO UPDATE SET
                   display_name      = excluded.display_name,
                   binary            = excluded.binary,
                   base_args         = excluded.base_args,
                   resume_args       = excluded.resume_args,
                   extra_args        = excluded.extra_args,
                   prompt_mode       = excluded.prompt_mode,
                   status            = excluded.status,
                   model_arg         = excluded.model_arg,
                   models_list_args  = excluded.models_list_args",
                params![spec.name, spec.display_name, spec.binary, base, resume, extra, prompt_mode, status, spec.model_arg, models_list],
            )
            .context("upserting agent")?;
        Ok(())
    }

    pub fn list_custom_agents(&self) -> Result<Vec<AgentSpec>> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, display_name, binary, base_args, resume_args, extra_args, prompt_mode, status, model_arg, models_list_args FROM agents")
            .context("preparing list_custom_agents")?;
        let rows = stmt
            .query_map([], Self::row_to_agent)
            .context("querying agents")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading agent row")??);
        }
        Ok(out)
    }

    pub fn delete_agent(&self, name: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM agents WHERE name = ?1", params![name])
            .context("deleting agent")?;
        Ok(())
    }

    /// Outer `rusqlite::Result` covers row-reading; inner `anyhow::Result`
    /// covers JSON / enum parsing. Custom agents always read back `builtin: false`.
    fn row_to_agent(row: &rusqlite::Row) -> rusqlite::Result<Result<AgentSpec>> {
        let base_json: String = row.get(3)?;
        let resume_json: String = row.get(4)?;
        let extra_json: String = row.get(5)?;
        let prompt_str: String = row.get(6)?;
        let status_str: String = row.get(7)?;
        let model_arg: Option<String> = row.get(8)?;
        let models_list_json: Option<String> = row.get(9)?;
        let parse = || -> Result<AgentSpec> {
            Ok(AgentSpec {
                name: row.get(0)?,
                display_name: row.get(1)?,
                binary: row.get(2)?,
                base_args: serde_json::from_str(&base_json).context("decoding base_args")?,
                resume_args: serde_json::from_str(&resume_json).context("decoding resume_args")?,
                extra_args: serde_json::from_str(&extra_json).context("decoding extra_args")?,
                auto_approve_args: vec![],
                prompt_mode: serde_json::from_value(serde_json::Value::String(prompt_str))
                    .context("decoding prompt_mode")?,
                status: serde_json::from_value(serde_json::Value::String(status_str))
                    .context("decoding status")?,
                model_arg,
                models_list_args: models_list_json
                    .filter(|s| !s.is_empty())
                    .map(|s| serde_json::from_str(&s).context("decoding models_list_args"))
                    .transpose()?,
                builtin: false,
                skill_injection: crate::agent::spec::SkillInjection::None,
            })
        };
        Ok(parse())
    }

    // ---- Tasks ----

    pub fn insert_task(&self, task: &Task) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO tasks (id, repo_id, title, worktree_path, branch, base_branch, status, created_at, updated_at, pr_number, pr_url, ticket_key, agent, model, setup_status, hidden, pending_prompt, auto_approve, in_place)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
                params![
                    task.id,
                    task.repo_id,
                    task.title,
                    task.worktree_path,
                    task.branch,
                    task.base_branch,
                    task.status.as_str(),
                    task.created_at,
                    task.updated_at,
                    task.pr_number,
                    task.pr_url,
                    task.ticket_key,
                    task.agent,
                    task.model,
                    task.setup_status.map(|s| s.as_str()),
                    task.hidden,
                    task.pending_prompt,
                    task.auto_approve,
                    task.in_place,
                ],
            )
            .context("inserting task")?;
        Ok(())
    }

    pub fn get_task(&self, id: &str) -> Result<Option<Task>> {
        let task = self
            .conn
            .query_row(
                "SELECT id, repo_id, title, worktree_path, branch, base_branch, status, created_at, updated_at, pr_number, pr_url, ticket_key, agent, model, setup_status, hidden, pending_prompt, auto_approve, in_place
                 FROM tasks WHERE id = ?1",
                params![id],
                Self::row_to_task,
            )
            .optional()
            .context("getting task")?;
        task.transpose()
    }

    pub fn list_tasks(&self) -> Result<Vec<Task>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, repo_id, title, worktree_path, branch, base_branch, status, created_at, updated_at, pr_number, pr_url, ticket_key, agent, model, setup_status, hidden, pending_prompt, auto_approve, in_place
                 FROM tasks",
            )
            .context("preparing list_tasks")?;
        let rows = stmt
            .query_map([], Self::row_to_task)
            .context("querying tasks")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading task row")??);
        }
        Ok(out)
    }

    pub fn list_tasks_for_repo(&self, repo_id: &str) -> Result<Vec<Task>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, repo_id, title, worktree_path, branch, base_branch, status, created_at, updated_at, pr_number, pr_url, ticket_key, agent, model, setup_status, hidden, pending_prompt, auto_approve, in_place
                 FROM tasks WHERE repo_id = ?1",
            )
            .context("preparing list_tasks_for_repo")?;
        let rows = stmt
            .query_map(params![repo_id], Self::row_to_task)
            .context("querying tasks for repo")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading task row")??);
        }
        Ok(out)
    }

    pub fn update_task_status(&self, id: &str, status: TaskStatus) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.conn
            .execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.as_str(), now, id],
            )
            .context("updating task status")?;
        Ok(())
    }

    /// Record the PR (number + url) associated with a task.
    pub fn set_task_pr(&self, id: &str, number: i64, url: &str) -> Result<()> {
        self.conn
            .execute(
                "UPDATE tasks SET pr_number = ?1, pr_url = ?2 WHERE id = ?3",
                params![number, url, id],
            )
            .context("setting task pr")?;
        Ok(())
    }

    /// Update a task's display title (and bump `updated_at`). Used by the
    /// agent-driven rename primitive (TASK-40); the caller validates/trims the
    /// name before persisting.
    pub fn update_task_title(&self, id: &str, title: &str) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.conn
            .execute(
                "UPDATE tasks SET title = ?1, updated_at = ?2 WHERE id = ?3",
                params![title, now, id],
            )
            .context("updating task title")?;
        Ok(())
    }

    /// Set (or clear) the agent remembered for a task.
    pub fn set_task_agent(&self, id: &str, agent: Option<&str>) -> Result<()> {
        self.conn
            .execute("UPDATE tasks SET agent = ?1 WHERE id = ?2", params![agent, id])
            .context("setting task agent")?;
        Ok(())
    }

    /// Set (or clear) the model remembered for a task.
    pub fn set_task_model(&self, task_id: &str, model: Option<&str>) -> Result<()> {
        self.conn
            .execute("UPDATE tasks SET model = ?2 WHERE id = ?1", params![task_id, model])
            .context("setting task model")?;
        Ok(())
    }

    /// Set (or clear) a task's auto-approve override. `None` ⇒ inherit the repo.
    pub fn set_task_auto_approve(&self, task_id: &str, value: Option<bool>) -> Result<()> {
        self.conn
            .execute("UPDATE tasks SET auto_approve = ?2 WHERE id = ?1", params![task_id, value])
            .context("setting task auto_approve")?;
        Ok(())
    }

    /// Set (or clear) a task's setup status. `None` clears it (no setup / skipped).
    pub fn set_task_setup_status(&self, id: &str, status: Option<SetupStatus>) -> Result<()> {
        self.conn
            .execute(
                "UPDATE tasks SET setup_status = ?1 WHERE id = ?2",
                params![status.map(|s| s.as_str()), id],
            )
            .context("setting task setup status")?;
        Ok(())
    }

    /// Set the hidden state of a task. Non-destructive visibility toggle.
    pub fn set_task_hidden(&self, id: &str, hidden: bool) -> Result<()> {
        self.conn
            .execute(
                "UPDATE tasks SET hidden = ?1 WHERE id = ?2",
                params![hidden as i32, id],
            )
            .context("setting task hidden")?;
        Ok(())
    }

    /// Set (or clear) a queued task's seeded launch prompt (TASK-90).
    pub fn set_task_pending_prompt(&self, id: &str, prompt: Option<&str>) -> Result<()> {
        self.conn
            .execute(
                "UPDATE tasks SET pending_prompt = ?1 WHERE id = ?2",
                params![prompt, id],
            )
            .context("setting task pending_prompt")?;
        Ok(())
    }

    /// Promote a queued (Pending) task to a live, launchable task: attach the
    /// freshly-created worktree + branch, flip status to Idle, and clear the
    /// seeded prompt (it's delivered via the task_launched event). TASK-90.
    pub fn promote_task(&self, id: &str, worktree_path: &str, branch: &str) -> Result<()> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.conn
            .execute(
                "UPDATE tasks SET worktree_path = ?1, branch = ?2, status = ?3,
                     pending_prompt = NULL, updated_at = ?4 WHERE id = ?5",
                params![worktree_path, branch, TaskStatus::Idle.as_str(), now, id],
            )
            .context("promoting pending task")?;
        Ok(())
    }

    /// Reconcile setups orphaned by a crash/restart: any task still marked
    /// `running` had its background setup job die with the process, so flip it
    /// to `failed` (an interrupted setup did not succeed). Called once at startup.
    pub fn reconcile_interrupted_setups(&self) -> Result<()> {
        self.conn
            .execute(
                "UPDATE tasks SET setup_status = ?1 WHERE setup_status = ?2",
                params![SetupStatus::Failed.as_str(), SetupStatus::Running.as_str()],
            )
            .context("reconciling interrupted setups")?;
        Ok(())
    }

    /// Set (or clear) a repo's default model.
    pub fn set_repo_default_model(&self, id: &str, model: Option<&str>) -> Result<()> {
        self.conn
            .execute("UPDATE repos SET default_model = ?1 WHERE id = ?2", params![model, id])
            .context("setting repo default model")?;
        Ok(())
    }

    pub fn delete_task(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM tasks WHERE id = ?1", params![id])
            .context("deleting task")?;
        Ok(())
    }

    // ---- Task dependencies (TASK-90) ----

    /// Queue `task_id` behind `depends_on_task_id`'s merge. Idempotent per edge.
    pub fn add_task_dependency(&self, task_id: &str, depends_on_task_id: &str) -> Result<()> {
        self.conn
            .execute(
                "INSERT OR IGNORE INTO task_dependencies (task_id, depends_on_task_id)
                 VALUES (?1, ?2)",
                params![task_id, depends_on_task_id],
            )
            .context("adding task dependency")?;
        Ok(())
    }

    /// Task ids queued on `dep_id` (i.e. with an edge → `dep_id`).
    pub fn dependents_of(&self, dep_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT task_id FROM task_dependencies WHERE depends_on_task_id = ?1")
            .context("preparing dependents_of")?;
        let rows = stmt
            .query_map(params![dep_id], |row| row.get::<_, String>(0))
            .context("querying dependents")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading dependent row")?);
        }
        Ok(out)
    }

    /// The `depends_on_task_id`s `task_id` is still queued behind (its unmet
    /// blockers). Mirror of `dependents_of`; used to describe a pending task's
    /// outstanding blockers to the UI (TASK-177).
    pub fn blockers_of(&self, task_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT depends_on_task_id FROM task_dependencies WHERE task_id = ?1")
            .context("preparing blockers_of")?;
        let rows = stmt
            .query_map(params![task_id], |row| row.get::<_, String>(0))
            .context("querying blockers")?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row.context("reading blocker row")?);
        }
        Ok(out)
    }

    /// Remove every edge pointing at `dep_id` (its merge satisfies them).
    pub fn remove_dependencies_on(&self, dep_id: &str) -> Result<()> {
        self.conn
            .execute(
                "DELETE FROM task_dependencies WHERE depends_on_task_id = ?1",
                params![dep_id],
            )
            .context("removing dependencies on merged task")?;
        Ok(())
    }

    /// Atomically read every dependent queued on `dep_id` AND remove those edges,
    /// in a single transaction, returning the dependents captured at removal time.
    ///
    /// TASK-182: this is the authoritative promotion set for `promote_dependents_of`.
    /// Folding the read and the remove into one lock/txn scope means an edge
    /// inserted by a concurrent `launch_task` (the queue-vs-finish window) is
    /// either fully seen-and-returned (so it gets promoted) or not yet inserted —
    /// never removed without being returned. Callers who peeked earlier (a cheap
    /// early-exit before an `await`) must promote THIS returned set, not the peek.
    pub fn take_dependents_on(&self, dep_id: &str) -> Result<Vec<String>> {
        let tx = self
            .conn
            .unchecked_transaction()
            .context("beginning take_dependents_on transaction")?;
        let dependents = {
            let mut stmt = tx
                .prepare("SELECT task_id FROM task_dependencies WHERE depends_on_task_id = ?1")
                .context("preparing take_dependents_on read")?;
            let rows = stmt
                .query_map(params![dep_id], |row| row.get::<_, String>(0))
                .context("querying dependents to take")?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.context("reading taken dependent row")?);
            }
            out
        };
        tx.execute(
            "DELETE FROM task_dependencies WHERE depends_on_task_id = ?1",
            params![dep_id],
        )
        .context("removing taken dependency edges")?;
        tx.commit().context("committing take_dependents_on")?;
        Ok(dependents)
    }

    /// Number of unmet dependency edges a task still has (0 ⇒ ready to promote).
    pub fn unmet_dependency_count(&self, task_id: &str) -> Result<i64> {
        let count: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM task_dependencies WHERE task_id = ?1",
                params![task_id],
                |row| row.get(0),
            )
            .context("counting unmet dependencies")?;
        Ok(count)
    }

    // ── Schedules (TASK-173) ──────────────────────────────────────────────────

    /// Column list for schedule reads, kept positionally in lockstep with
    /// `map_schedule_row`'s `row.get(0..=14)`. Shared by every read query so
    /// the order can't drift between them.
    const SCHEDULE_COLS: &str = "id, repo_id, name, prompt, cron, agent, model, base_branch, enabled, next_run_at, last_run_at, created_at, updated_at, one_shot, skip_repo_prompt";

    fn map_schedule_row(row: &rusqlite::Row) -> rusqlite::Result<Schedule> {
        Ok(Schedule {
            id: row.get(0)?,
            repo_id: row.get(1)?,
            name: row.get(2)?,
            prompt: row.get(3)?,
            cron: row.get(4)?,
            agent: row.get(5)?,
            model: row.get(6)?,
            base_branch: row.get(7)?,
            enabled: row.get(8)?,
            next_run_at: row.get(9)?,
            last_run_at: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
            one_shot: row.get(13)?,
            skip_repo_prompt: row.get(14)?,
        })
    }

    pub fn insert_schedule(&self, s: &Schedule) -> Result<()> {
        self.conn
            .execute(
                "INSERT INTO schedules (id, repo_id, name, prompt, cron, agent, model, base_branch, enabled, next_run_at, last_run_at, created_at, updated_at, one_shot, skip_repo_prompt)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    s.id, s.repo_id, s.name, s.prompt, s.cron, s.agent, s.model, s.base_branch,
                    s.enabled, s.next_run_at, s.last_run_at, s.created_at, s.updated_at, s.one_shot,
                    s.skip_repo_prompt
                ],
            )
            .context("inserting schedule")?;
        Ok(())
    }

    pub fn list_schedules(&self, repo_id: &str) -> Result<Vec<Schedule>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {} FROM schedules WHERE repo_id = ?1 ORDER BY created_at ASC",
                Self::SCHEDULE_COLS
            ))
            .context("preparing list_schedules")?;
        let rows = stmt
            .query_map(params![repo_id], Self::map_schedule_row)
            .context("querying schedules")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("reading schedule row")?);
        }
        Ok(out)
    }

    pub fn get_schedule(&self, id: &str) -> Result<Option<Schedule>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {} FROM schedules WHERE id = ?1",
                Self::SCHEDULE_COLS
            ))
            .context("preparing get_schedule")?;
        let mut rows = stmt
            .query_map(params![id], Self::map_schedule_row)
            .context("querying schedule")?;
        match rows.next() {
            Some(r) => Ok(Some(r.context("reading schedule row")?)),
            None => Ok(None),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn update_schedule_fields(
        &self,
        id: &str,
        name: &str,
        prompt: &str,
        cron: &str,
        agent: Option<&str>,
        model: Option<&str>,
        base_branch: Option<&str>,
        enabled: bool,
        skip_repo_prompt: bool,
        next_run_at: Option<i64>,
        updated_at: i64,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE schedules SET name = ?1, prompt = ?2, cron = ?3, agent = ?4, model = ?5,
                 base_branch = ?6, enabled = ?7, skip_repo_prompt = ?8, next_run_at = ?9, updated_at = ?10 WHERE id = ?11",
                params![name, prompt, cron, agent, model, base_branch, enabled, skip_repo_prompt, next_run_at, updated_at, id],
            )
            .context("updating schedule")?;
        Ok(())
    }

    pub fn delete_schedule(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM schedules WHERE id = ?1", params![id])
            .context("deleting schedule")?;
        Ok(())
    }

    pub fn due_schedules(&self, now: i64) -> Result<Vec<Schedule>> {
        let mut stmt = self
            .conn
            .prepare(&format!(
                "SELECT {} FROM schedules
                 WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1
                 ORDER BY next_run_at ASC",
                Self::SCHEDULE_COLS
            ))
            .context("preparing due_schedules")?;
        let rows = stmt
            .query_map(params![now], Self::map_schedule_row)
            .context("querying due schedules")?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r.context("reading schedule row")?);
        }
        Ok(out)
    }

    pub fn advance_schedule(
        &self,
        id: &str,
        next_run_at: Option<i64>,
        last_run_at: i64,
        updated_at: i64,
    ) -> Result<()> {
        self.conn
            .execute(
                "UPDATE schedules SET next_run_at = ?1, last_run_at = ?2, updated_at = ?3 WHERE id = ?4",
                params![next_run_at, last_run_at, updated_at, id],
            )
            .context("advancing schedule")?;
        Ok(())
    }

    /// Retire a one-shot schedule after it has fired: disable it and clear its
    /// next fire time so it is never re-selected by `due_schedules`, while
    /// recording when it ran. TASK-179.
    pub fn retire_schedule(&self, id: &str, last_run_at: i64, updated_at: i64) -> Result<()> {
        self.conn
            .execute(
                "UPDATE schedules SET enabled = 0, next_run_at = NULL, last_run_at = ?1, updated_at = ?2 WHERE id = ?3",
                params![last_run_at, updated_at, id],
            )
            .context("retiring schedule")?;
        Ok(())
    }

    /// Maps a row to a `Result<Task>`, where the outer `rusqlite::Result` covers
    /// row-reading errors and the inner `anyhow::Result` covers status parsing.
    fn row_to_task(row: &rusqlite::Row) -> rusqlite::Result<Result<Task>> {
        let status_str: String = row.get(6)?;
        // TASK-167: degrade gracefully on an unknown/future TaskStatus (e.g. a
        // row written by a newer La Vigie sharing this store) instead of failing
        // the whole list — map it to Idle (harmless render) and log a warning.
        let status = TaskStatus::from_str(&status_str).unwrap_or_else(|_| {
            eprintln!(
                "TASK-167: unknown TaskStatus {status_str:?} in task row {:?}; defaulting to Idle",
                row.get::<_, String>(0).unwrap_or_default()
            );
            TaskStatus::Idle
        });
        let setup_status = match row.get::<_, Option<String>>(14)? {
            Some(s) => match SetupStatus::from_str(&s) {
                Ok(v) => Some(v),
                Err(e) => return Ok(Err(e)),
            },
            None => None,
        };
        Ok(Ok(Task {
            id: row.get(0)?,
            repo_id: row.get(1)?,
            title: row.get(2)?,
            worktree_path: row.get(3)?,
            branch: row.get(4)?,
            base_branch: row.get(5)?,
            status,
            created_at: row.get(7)?,
            updated_at: row.get(8)?,
            pr_number: row.get(9)?,
            pr_url: row.get(10)?,
            ticket_key: row.get(11)?,
            agent: row.get(12)?,
            model: row.get(13)?,
            setup_status,
            hidden: row.get(15)?,
            pending_prompt: row.get(16)?,
            auto_approve: row.get(17)?,
            in_place: row.get(18)?,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn open_test_store() -> (TempDir, TaskStore) {
        let dir = TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test.db");
        let store = TaskStore::open(&db_path).expect("open store");
        (dir, store)
    }

    fn sample_repo(id: &str) -> Repo {
        Repo {
            id: id.to_string(),
            name: "my-repo".to_string(),
            path: "/tmp/my-repo".to_string(),
            default_branch: "main".to_string(),
            remote_url: Some("git@github.com:example/my-repo.git".to_string()),
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
        }
    }

    fn sample_task(id: &str, repo_id: &str) -> Task {
        Task {
            id: id.to_string(),
            repo_id: repo_id.to_string(),
            title: "Do the thing".to_string(),
            worktree_path: "/tmp/my-repo-worktrees/task-1".to_string(),
            branch: "task/do-the-thing".to_string(),
            base_branch: "main".to_string(),
            status: TaskStatus::Idle,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
            pr_number: None,
            pr_url: None,
            ticket_key: None,
            agent: None,
            model: None,
            setup_status: None,
            hidden: false,
            pending_prompt: None,
            auto_approve: None,
            in_place: false,
        }
    }

    #[test]
    fn task_round_trips_with_agent() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        let mut t = sample_task("task-agent", "repo-1");
        t.agent = Some("aider".to_string());
        store.insert_task(&t).unwrap();
        assert_eq!(store.get_task("task-agent").unwrap(), Some(t));
    }

    #[test]
    fn set_task_agent_updates_and_clears() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_task(&sample_task("task-1", "repo-1")).unwrap();
        store.set_task_agent("task-1", Some("codex")).unwrap();
        assert_eq!(store.get_task("task-1").unwrap().unwrap().agent, Some("codex".to_string()));
        store.set_task_agent("task-1", None).unwrap();
        assert_eq!(store.get_task("task-1").unwrap().unwrap().agent, None);
    }

    #[test]
    fn update_task_title_changes_title_and_bumps_updated_at() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        let t = sample_task("task-1", "repo-1");
        let old_updated = t.updated_at;
        store.insert_task(&t).unwrap();
        store.update_task_title("task-1", "Renamed by agent").unwrap();
        let got = store.get_task("task-1").unwrap().unwrap();
        assert_eq!(got.title, "Renamed by agent");
        assert!(got.updated_at >= old_updated);
    }

    #[test]
    fn repo_round_trips_with_default_agent() {
        let (_dir, store) = open_test_store();
        let mut r = sample_repo("repo-da");
        r.default_agent = Some("antigravity".to_string());
        store.insert_repo(&r).unwrap();
        assert_eq!(store.get_repo("repo-da").unwrap(), Some(r));
    }

    #[test]
    fn update_repo_sets_and_clears_default_agent() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store
            .update_repo("repo-1", "my-repo", "main", None, None, false, None, Some("aider"))
            .unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().default_agent, Some("aider".to_string()));
        store
            .update_repo("repo-1", "my-repo", "main", None, None, false, None, None)
            .unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().default_agent, None);
    }

    #[test]
    fn repo_round_trips_with_default_model() {
        let (_dir, store) = open_test_store();
        let mut r = sample_repo("repo-dm");
        r.default_agent = Some("opencode".to_string());
        r.default_model = Some("zhipuai-coding-plan/glm-5.2".to_string());
        store.insert_repo(&r).unwrap();
        assert_eq!(store.get_repo("repo-dm").unwrap(), Some(r));
    }

    #[test]
    fn set_repo_default_model_updates_and_clears() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.set_repo_default_model("repo-1", Some("opencode/glm-4.7-free")).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().default_model, Some("opencode/glm-4.7-free".to_string()));
        store.set_repo_default_model("repo-1", None).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().default_model, None);
    }

    #[test]
    fn open_migrates_old_db_without_agent_columns() {
        let dir = TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("old.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "
                CREATE TABLE repos (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
                    default_branch TEXT NOT NULL, remote_url TEXT, worktree_root TEXT, setup_command TEXT
                );
                CREATE TABLE tasks (
                    id TEXT PRIMARY KEY, repo_id TEXT NOT NULL, title TEXT NOT NULL,
                    worktree_path TEXT NOT NULL, branch TEXT NOT NULL, base_branch TEXT NOT NULL,
                    status TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                    pr_number INTEGER, pr_url TEXT, ticket_key TEXT,
                    FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE
                );
                INSERT INTO repos VALUES ('repo-1','r','/tmp/r','main',NULL,NULL,NULL);
                INSERT INTO tasks VALUES ('task-old','repo-1','t','/tmp/wt','b','main','idle',1,1,NULL,NULL,NULL);
                ",
            )
            .unwrap();
        }
        let store = TaskStore::open(&db_path).expect("open + migrate old db");
        assert_eq!(store.get_task("task-old").unwrap().unwrap().agent, None);
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().default_agent, None);

        let mut t = sample_task("task-new", "repo-1");
        t.agent = Some("aider".to_string());
        store.insert_task(&t).unwrap();
        assert_eq!(store.get_task("task-new").unwrap().unwrap().agent, Some("aider".to_string()));
    }

    #[test]
    fn open_creates_usable_empty_store() {
        let (_dir, store) = open_test_store();
        assert_eq!(store.list_repos().unwrap(), vec![]);
        assert_eq!(store.list_tasks().unwrap(), vec![]);
    }

    #[test]
    fn repo_round_trips_all_fields_with_remote_url() {
        let (_dir, store) = open_test_store();
        let repo = sample_repo("repo-1");
        store.insert_repo(&repo).unwrap();

        let fetched = store.get_repo("repo-1").unwrap();
        assert_eq!(fetched, Some(repo.clone()));

        let listed = store.list_repos().unwrap();
        assert_eq!(listed, vec![repo]);
    }

    #[test]
    fn repo_round_trips_with_no_remote_url() {
        let (_dir, store) = open_test_store();
        let mut repo = sample_repo("repo-2");
        repo.remote_url = None;
        store.insert_repo(&repo).unwrap();

        let fetched = store.get_repo("repo-2").unwrap();
        assert_eq!(fetched, Some(repo));
    }

    #[test]
    fn get_repo_returns_none_when_missing() {
        let (_dir, store) = open_test_store();
        assert_eq!(store.get_repo("nope").unwrap(), None);
    }

    #[test]
    fn task_round_trips_all_fields_and_status() {
        let (_dir, store) = open_test_store();
        let repo = sample_repo("repo-1");
        store.insert_repo(&repo).unwrap();

        let mut task = sample_task("task-1", "repo-1");
        task.status = TaskStatus::NeedsAttention;
        store.insert_task(&task).unwrap();

        let fetched = store.get_task("task-1").unwrap();
        assert_eq!(fetched, Some(task.clone()));

        let listed = store.list_tasks().unwrap();
        assert_eq!(listed, vec![task]);
    }

    #[test]
    fn list_tasks_tolerates_unknown_status_and_keeps_loading() {
        // TASK-167: a row carrying an unknown/future TaskStatus (e.g. written by a
        // newer La Vigie sharing this store) must not brick the whole list.
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_task(&sample_task("task-good", "repo-1")).unwrap();
        store.insert_task(&sample_task("task-weird", "repo-1")).unwrap();

        // Corrupt one row's status to a value this binary can't parse.
        store
            .conn
            .execute(
                "UPDATE tasks SET status = 'from_the_future' WHERE id = 'task-weird'",
                [],
            )
            .unwrap();

        let listed = store.list_tasks().unwrap();
        // The whole list still loads — the unknown row is present, not dropped.
        assert_eq!(listed.len(), 2);
        let weird = listed.iter().find(|t| t.id == "task-weird").unwrap();
        assert_eq!(weird.status, TaskStatus::Idle);
        let good = listed.iter().find(|t| t.id == "task-good").unwrap();
        assert_eq!(good.status, TaskStatus::Idle);

        // get_task on the corrupted row degrades gracefully too.
        assert_eq!(store.get_task("task-weird").unwrap().unwrap().status, TaskStatus::Idle);
    }

    #[test]
    fn list_tasks_for_repo_returns_only_that_repos_tasks() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_repo(&sample_repo("repo-2")).unwrap();

        let task_a = sample_task("task-a", "repo-1");
        let task_b = sample_task("task-b", "repo-1");
        let task_c = sample_task("task-c", "repo-2");
        store.insert_task(&task_a).unwrap();
        store.insert_task(&task_b).unwrap();
        store.insert_task(&task_c).unwrap();

        let mut repo1_tasks = store.list_tasks_for_repo("repo-1").unwrap();
        repo1_tasks.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(repo1_tasks, vec![task_a, task_b]);

        let repo2_tasks = store.list_tasks_for_repo("repo-2").unwrap();
        assert_eq!(repo2_tasks, vec![task_c]);
    }

    #[test]
    fn update_task_status_changes_status_and_bumps_updated_at() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        let task = sample_task("task-1", "repo-1");
        store.insert_task(&task).unwrap();

        store
            .update_task_status("task-1", TaskStatus::Working)
            .unwrap();

        let fetched = store.get_task("task-1").unwrap().unwrap();
        assert_eq!(fetched.status, TaskStatus::Working);
        assert!(fetched.updated_at >= task.updated_at);
    }

    #[test]
    fn delete_task_removes_one_task() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_task(&sample_task("task-1", "repo-1")).unwrap();
        store.insert_task(&sample_task("task-2", "repo-1")).unwrap();

        store.delete_task("task-1").unwrap();

        assert_eq!(store.get_task("task-1").unwrap(), None);
        assert!(store.get_task("task-2").unwrap().is_some());
    }

    #[test]
    fn delete_repo_cascades_and_removes_its_tasks() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_repo(&sample_repo("repo-2")).unwrap();
        store.insert_task(&sample_task("task-1", "repo-1")).unwrap();
        store.insert_task(&sample_task("task-2", "repo-2")).unwrap();

        store.delete_repo("repo-1").unwrap();

        assert_eq!(store.get_repo("repo-1").unwrap(), None);
        assert!(store.get_repo("repo-2").unwrap().is_some());
        assert_eq!(store.list_tasks_for_repo("repo-1").unwrap(), vec![]);
        assert!(store.get_task("task-1").unwrap().is_none());
        assert!(store.get_task("task-2").unwrap().is_some());
    }

    #[test]
    fn task_round_trips_pr_fields() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();

        let mut task = sample_task("task-pr", "repo-1");
        task.pr_number = Some(42);
        task.pr_url = Some("https://github.com/example/repo/pull/42".to_string());
        store.insert_task(&task).unwrap();

        let fetched = store.get_task("task-pr").unwrap().unwrap();
        assert_eq!(fetched.pr_number, Some(42));
        assert_eq!(
            fetched.pr_url.as_deref(),
            Some("https://github.com/example/repo/pull/42")
        );

        // A task with no PR round-trips as None.
        store.insert_task(&sample_task("task-none", "repo-1")).unwrap();
        let none = store.get_task("task-none").unwrap().unwrap();
        assert_eq!(none.pr_number, None);
        assert_eq!(none.pr_url, None);
    }

    #[test]
    fn set_task_pr_records_number_and_url() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_task(&sample_task("task-1", "repo-1")).unwrap();

        store
            .set_task_pr("task-1", 7, "https://github.com/example/repo/pull/7")
            .unwrap();

        let fetched = store.get_task("task-1").unwrap().unwrap();
        assert_eq!(fetched.pr_number, Some(7));
        assert_eq!(
            fetched.pr_url.as_deref(),
            Some("https://github.com/example/repo/pull/7")
        );
    }

    #[test]
    fn pending_prompt_round_trips_and_migrates() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("v.sqlite3");
        let store = TaskStore::open(&db_path).unwrap();
        let repo = sample_repo("r1");
        store.insert_repo(&repo).unwrap();

        let mut task = sample_task("t1", "r1");
        task.pending_prompt = Some("do the thing".to_string());
        store.insert_task(&task).unwrap();
        assert_eq!(
            store.get_task("t1").unwrap().unwrap().pending_prompt.as_deref(),
            Some("do the thing")
        );

        store.set_task_pending_prompt("t1", None).unwrap();
        assert_eq!(store.get_task("t1").unwrap().unwrap().pending_prompt, None);
    }

    #[test]
    fn open_migrates_old_db_without_pr_columns() {
        let dir = TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("old.db");

        // Build a DB with the pre-PR schema and one row, by hand.
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "
                CREATE TABLE repos (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
                    default_branch TEXT NOT NULL, remote_url TEXT
                );
                CREATE TABLE tasks (
                    id TEXT PRIMARY KEY, repo_id TEXT NOT NULL, title TEXT NOT NULL,
                    worktree_path TEXT NOT NULL, branch TEXT NOT NULL, base_branch TEXT NOT NULL,
                    status TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                    FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE
                );
                INSERT INTO repos VALUES ('repo-1','r','/tmp/r','main',NULL);
                INSERT INTO tasks VALUES ('task-old','repo-1','t','/tmp/wt','b','main','idle',1,1);
                ",
            )
            .unwrap();
        }

        // Opening via TaskStore must migrate (add pr columns) without data loss.
        let store = TaskStore::open(&db_path).expect("open + migrate old db");
        let old = store.get_task("task-old").unwrap().unwrap();
        assert_eq!(old.pr_number, None);
        assert_eq!(old.pr_url, None);
        assert_eq!(old.title, "t");

        // And a new task with a PR round-trips on the migrated DB.
        let mut t = sample_task("task-new", "repo-1");
        t.pr_number = Some(99);
        t.pr_url = Some("https://github.com/example/repo/pull/99".to_string());
        store.insert_task(&t).unwrap();
        let fetched = store.get_task("task-new").unwrap().unwrap();
        assert_eq!(fetched.pr_number, Some(99));
    }

    #[test]
    fn task_status_serde_round_trips_to_exact_snake_case_strings() {
        assert_eq!(
            serde_json::to_string(&TaskStatus::Idle).unwrap(),
            "\"idle\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Working).unwrap(),
            "\"working\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::NeedsAttention).unwrap(),
            "\"needs_attention\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Done).unwrap(),
            "\"done\""
        );
        assert_eq!(
            serde_json::to_string(&TaskStatus::Error).unwrap(),
            "\"error\""
        );

        assert_eq!(
            serde_json::from_str::<TaskStatus>("\"needs_attention\"").unwrap(),
            TaskStatus::NeedsAttention
        );
        assert_eq!(
            serde_json::from_str::<TaskStatus>("\"idle\"").unwrap(),
            TaskStatus::Idle
        );
    }

    #[test]
    fn task_status_as_str_and_from_str_round_trip() {
        let all = [
            TaskStatus::Idle,
            TaskStatus::Working,
            TaskStatus::NeedsAttention,
            TaskStatus::Done,
            TaskStatus::Error,
        ];
        for status in all {
            let s = status.as_str();
            assert_eq!(TaskStatus::from_str(s).unwrap(), status);
        }
    }

    #[test]
    fn update_repo_changes_name_and_default_branch_only() {
        let (_dir, store) = open_test_store();
        let repo = sample_repo("repo-1");
        store.insert_repo(&repo).unwrap();

        store.update_repo("repo-1", "renamed", "develop", None, None, false, None, None).unwrap();

        let got = store.get_repo("repo-1").unwrap().unwrap();
        assert_eq!(got.name, "renamed");
        assert_eq!(got.default_branch, "develop");
        assert_eq!(got.path, repo.path);
        assert_eq!(got.remote_url, repo.remote_url);
    }

    #[test]
    fn update_repo_errors_when_id_missing() {
        let (_dir, store) = open_test_store();
        let err = store
            .update_repo("nope", "x", "main", None, None, false, None, None)
            .expect_err("updating a missing repo should error");
        assert!(err.to_string().contains("repo not found"));
    }

    #[test]
    fn update_repo_sets_and_clears_worktree_root() {
        let (_dir, store) = open_test_store();
        let repo = sample_repo("repo-1");
        store.insert_repo(&repo).unwrap();

        store.update_repo("repo-1", "my-repo", "main", Some("/tmp/wt"), None, false, None, None).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().worktree_root, Some("/tmp/wt".to_string()));

        store.update_repo("repo-1", "my-repo", "main", None, None, false, None, None).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().worktree_root, None);
    }

    #[test]
    fn update_repo_sets_and_clears_setup_command() {
        let (_dir, store) = open_test_store();
        let repo = sample_repo("repo-1");
        store.insert_repo(&repo).unwrap();

        store.update_repo("repo-1", "my-repo", "main", None, Some("cwt"), false, None, None).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().setup_command, Some("cwt".to_string()));

        store.update_repo("repo-1", "my-repo", "main", None, None, false, None, None).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().setup_command, None);
    }

    #[test]
    fn repo_round_trips_with_worktree_root() {
        let (_dir, store) = open_test_store();
        let mut repo = sample_repo("repo-wt");
        repo.worktree_root = Some("/Users/me/work/wt".to_string());
        store.insert_repo(&repo).unwrap();

        let fetched = store.get_repo("repo-wt").unwrap();
        assert_eq!(fetched, Some(repo));
    }

    #[test]
    fn repo_round_trips_with_setup_command() {
        let (_dir, store) = open_test_store();
        let mut repo = sample_repo("repo-sc");
        repo.setup_command = Some("cwt".to_string());
        store.insert_repo(&repo).unwrap();

        let fetched = store.get_repo("repo-sc").unwrap();
        assert_eq!(fetched, Some(repo));
    }

    #[test]
    fn open_migrates_old_repos_without_setup_command() {
        let dir = TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("old.db");
        // Simulate a DB created before the setup_command column existed
        // (worktree_root is present — this is a post-TASK-25 schema).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE repos (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
                    default_branch TEXT NOT NULL, remote_url TEXT, worktree_root TEXT
                );
                INSERT INTO repos (id, name, path, default_branch, remote_url, worktree_root)
                VALUES ('r1', 'old', '/tmp/old', 'main', NULL, NULL);",
            )
            .unwrap();
        }
        let store = TaskStore::open(&db_path).expect("open + migrate old repos db");
        let fetched = store.get_repo("r1").unwrap().unwrap();
        assert_eq!(fetched.setup_command, None);

        let mut repo = sample_repo("r2");
        repo.setup_command = Some("make setup".to_string());
        store.insert_repo(&repo).unwrap();
        assert_eq!(store.get_repo("r2").unwrap().unwrap().setup_command, Some("make setup".to_string()));
    }

    #[test]
    fn open_migrates_old_repos_without_worktree_root() {
        let dir = TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("old.db");
        // Simulate a DB created before the worktree_root column existed.
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE repos (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
                    default_branch TEXT NOT NULL, remote_url TEXT
                );
                INSERT INTO repos (id, name, path, default_branch, remote_url)
                VALUES ('r1', 'old', '/tmp/old', 'main', NULL);",
            )
            .unwrap();
        }
        // Opening via TaskStore must add the column without data loss.
        let store = TaskStore::open(&db_path).expect("open + migrate old repos db");
        let fetched = store.get_repo("r1").unwrap().unwrap();
        assert_eq!(fetched.worktree_root, None);

        // And a new repo with a worktree_root round-trips on the migrated DB.
        let mut repo = sample_repo("r2");
        repo.worktree_root = Some("/tmp/custom".to_string());
        store.insert_repo(&repo).unwrap();
        assert_eq!(store.get_repo("r2").unwrap().unwrap().worktree_root, Some("/tmp/custom".to_string()));
    }

    fn sample_agent(name: &str) -> crate::agent::spec::AgentSpec {
        use crate::agent::spec::{PromptMode, StatusMechanism};
        crate::agent::spec::AgentSpec {
            name: name.to_string(),
            display_name: "My Agent".to_string(),
            binary: "my-agent".to_string(),
            base_args: vec!["--foo".to_string()],
            resume_args: vec![],
            extra_args: vec!["--model".to_string(), "x".to_string()],
            auto_approve_args: vec![],
            prompt_mode: PromptMode::Arg,
            status: StatusMechanism::Lifecycle,
            model_arg: None,
            models_list_args: None,
            builtin: false,
            skill_injection: crate::agent::spec::SkillInjection::None,
        }
    }

    #[test]
    fn custom_agent_round_trips() {
        let (_dir, store) = open_test_store();
        let mut agent = sample_agent("my-agent");
        agent.model_arg = Some("--model".into());
        agent.models_list_args = Some(vec!["models".into()]);
        store.upsert_agent(&agent).unwrap();
        let got = store.list_custom_agents().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].model_arg.as_deref(), Some("--model"));
        assert_eq!(got[0].models_list_args, Some(vec!["models".to_string()]));
        assert_eq!(got[0], agent);
    }

    #[test]
    fn upsert_agent_replaces_by_name() {
        let (_dir, store) = open_test_store();
        store.upsert_agent(&sample_agent("a")).unwrap();
        let mut updated = sample_agent("a");
        updated.display_name = "Renamed".to_string();
        store.upsert_agent(&updated).unwrap();
        let all = store.list_custom_agents().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].display_name, "Renamed");
    }

    #[test]
    fn delete_agent_removes_one() {
        let (_dir, store) = open_test_store();
        store.upsert_agent(&sample_agent("a")).unwrap();
        store.upsert_agent(&sample_agent("b")).unwrap();
        store.delete_agent("a").unwrap();
        let names: Vec<String> = store.list_custom_agents().unwrap().into_iter().map(|s| s.name).collect();
        assert_eq!(names, vec!["b".to_string()]);
    }

    #[test]
    fn app_setting_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("v.db")).unwrap();
        assert_eq!(store.get_app_setting("sound_notifications").unwrap(), None);
        store.set_app_setting("sound_notifications", r#"{"muted":true}"#).unwrap();
        assert_eq!(
            store.get_app_setting("sound_notifications").unwrap().as_deref(),
            Some(r#"{"muted":true}"#)
        );
        // upsert overwrites
        store.set_app_setting("sound_notifications", r#"{"muted":false}"#).unwrap();
        assert_eq!(
            store.get_app_setting("sound_notifications").unwrap().as_deref(),
            Some(r#"{"muted":false}"#)
        );
    }

    #[test]
    fn repo_sound_settings_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("v.db")).unwrap();
        let repo = Repo {
            id: "r1".into(), name: "n".into(), path: "/p".into(),
            default_branch: "main".into(), remote_url: None, worktree_root: None,
            setup_command: None, default_agent: None, auto_start_agent: false,
            initial_prompt: None, default_model: None, sound_settings: None, fetch_remote_base: None,
            auto_approve: None, in_place_default: false,
        };
        store.insert_repo(&repo).unwrap();
        assert_eq!(store.get_repo("r1").unwrap().unwrap().sound_settings, None);
        store.set_repo_sound_settings("r1", Some(r#"{"muted":true}"#)).unwrap();
        assert_eq!(
            store.get_repo("r1").unwrap().unwrap().sound_settings.as_deref(),
            Some(r#"{"muted":true}"#)
        );
        store.set_repo_sound_settings("r1", None).unwrap();
        assert_eq!(store.get_repo("r1").unwrap().unwrap().sound_settings, None);
    }

    #[test]
    fn migrates_old_db_adding_sound_settings_and_app_settings() {
        // Pre-TASK-78 schema: repos without sound_settings, no app_settings table.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("old.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE repos (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
                    default_branch TEXT NOT NULL, remote_url TEXT, worktree_root TEXT,
                    setup_command TEXT, default_agent TEXT,
                    auto_start_agent INTEGER NOT NULL DEFAULT 0, initial_prompt TEXT
                );
                INSERT INTO repos (id,name,path,default_branch,auto_start_agent)
                VALUES ('r1','n','/p','main',0);",
            ).unwrap();
        }
        let store = TaskStore::open(&path).unwrap();
        // Old repo reads back with sound_settings = None (column added by migration).
        assert_eq!(store.get_repo("r1").unwrap().unwrap().sound_settings, None);
        // app_settings table now exists and is usable.
        store.set_app_setting("k", "v").unwrap();
        assert_eq!(store.get_app_setting("k").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn list_custom_agents_always_reads_builtin_false() {
        let (_dir, store) = open_test_store();
        let mut a = sample_agent("a");
        a.builtin = true; // even if a caller sets true, storage normalizes to false on read
        store.upsert_agent(&a).unwrap();
        assert!(!store.list_custom_agents().unwrap()[0].builtin);
    }

    #[test]
    fn task_round_trips_with_ticket_key() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();

        let mut t = sample_task("task-key", "repo-1");
        t.ticket_key = Some("TST-1".to_string());
        store.insert_task(&t).unwrap();

        assert_eq!(store.get_task("task-key").unwrap(), Some(t));
    }

    #[test]
    fn task_round_trips_without_ticket_key() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();

        let t = sample_task("task-nokey", "repo-1"); // ticket_key: None
        store.insert_task(&t).unwrap();

        assert_eq!(store.get_task("task-nokey").unwrap().unwrap().ticket_key, None);
    }

    #[test]
    fn repo_round_trips_with_fetch_remote_base() {
        let (_dir, store) = open_test_store();
        let mut r = sample_repo("repo-frb");
        r.fetch_remote_base = Some(true);
        store.insert_repo(&r).unwrap();
        assert_eq!(store.get_repo("repo-frb").unwrap(), Some(r));
    }

    #[test]
    fn set_repo_fetch_remote_base_updates_and_clears() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.set_repo_fetch_remote_base("repo-1", Some(false)).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().fetch_remote_base, Some(false));
        store.set_repo_fetch_remote_base("repo-1", None).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().fetch_remote_base, None);
    }

    #[test]
    fn repo_round_trips_with_auto_approve() {
        let (_dir, store) = open_test_store();
        let mut r = sample_repo("repo-1");
        r.auto_approve = Some(false);
        store.insert_repo(&r).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().auto_approve, Some(false));
    }

    #[test]
    fn set_repo_auto_approve_updates_and_clears() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.set_repo_auto_approve("repo-1", Some(false)).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().auto_approve, Some(false));
        store.set_repo_auto_approve("repo-1", None).unwrap();
        assert_eq!(store.get_repo("repo-1").unwrap().unwrap().auto_approve, None);
    }

    #[test]
    fn task_round_trips_with_auto_approve() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        let mut t = sample_task("task-1", "repo-1");
        t.auto_approve = Some(true);
        store.insert_task(&t).unwrap();
        assert_eq!(store.get_task("task-1").unwrap().unwrap().auto_approve, Some(true));
    }

    #[test]
    fn set_task_auto_approve_updates_and_clears() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_task(&sample_task("task-1", "repo-1")).unwrap();
        store.set_task_auto_approve("task-1", Some(false)).unwrap();
        assert_eq!(store.get_task("task-1").unwrap().unwrap().auto_approve, Some(false));
        store.set_task_auto_approve("task-1", None).unwrap();
        assert_eq!(store.get_task("task-1").unwrap().unwrap().auto_approve, None);
    }

    #[test]
    fn repo_round_trips_auto_start_and_initial_prompt() {
        let (_dir, store) = open_test_store();
        let mut r = sample_repo("repo-ap");
        r.auto_start_agent = true;
        r.initial_prompt = Some("Read CLAUDE.md first".to_string());
        store.insert_repo(&r).unwrap();
        let got = store.get_repo("repo-ap").unwrap().unwrap();
        assert!(got.auto_start_agent);
        assert_eq!(got.initial_prompt, Some("Read CLAUDE.md first".to_string()));
    }

    #[test]
    fn update_repo_persists_auto_start_and_initial_prompt() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store
            .update_repo("repo-1", "my-repo", "main", None, None, true, Some("hello"), None)
            .unwrap();
        let got = store.get_repo("repo-1").unwrap().unwrap();
        assert!(got.auto_start_agent);
        assert_eq!(got.initial_prompt, Some("hello".to_string()));
        // Clearing works.
        store
            .update_repo("repo-1", "my-repo", "main", None, None, false, None, None)
            .unwrap();
        let got = store.get_repo("repo-1").unwrap().unwrap();
        assert!(!got.auto_start_agent);
        assert_eq!(got.initial_prompt, None);
    }

    #[test]
    fn task_model_round_trips_and_defaults_none() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        let task = sample_task("task-1", "repo-1");
        store.insert_task(&task).unwrap();
        assert_eq!(store.get_task("task-1").unwrap().unwrap().model, None);
        store.set_task_model("task-1", Some("zhipuai-coding-plan/glm-5.2")).unwrap();
        assert_eq!(
            store.get_task("task-1").unwrap().unwrap().model.as_deref(),
            Some("zhipuai-coding-plan/glm-5.2")
        );
        store.set_task_model("task-1", None).unwrap();
        assert_eq!(store.get_task("task-1").unwrap().unwrap().model, None);
    }

    #[test]
    fn custom_agent_round_trips_model_fields() {
        let (_dir, store) = open_test_store();
        let mut spec = sample_agent("my-oc");
        spec.model_arg = Some("--model".into());
        spec.models_list_args = Some(vec!["models".into()]);
        store.upsert_agent(&spec).unwrap();
        let got = store.list_custom_agents().unwrap().into_iter().find(|s| s.name == "my-oc").unwrap();
        assert_eq!(got.model_arg.as_deref(), Some("--model"));
        assert_eq!(got.models_list_args, Some(vec!["models".to_string()]));
    }

    #[test]
    fn open_migrates_old_db_without_ticket_key() {
        let dir = TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("old.db");

        // A DB whose tasks table predates ticket_key (has the pr columns).
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "
                CREATE TABLE repos (
                    id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
                    default_branch TEXT NOT NULL, remote_url TEXT
                );
                CREATE TABLE tasks (
                    id TEXT PRIMARY KEY, repo_id TEXT NOT NULL, title TEXT NOT NULL,
                    worktree_path TEXT NOT NULL, branch TEXT NOT NULL, base_branch TEXT NOT NULL,
                    status TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                    pr_number INTEGER, pr_url TEXT,
                    FOREIGN KEY (repo_id) REFERENCES repos(id) ON DELETE CASCADE
                );
                INSERT INTO repos VALUES ('repo-1','r','/tmp/r','main',NULL);
                INSERT INTO tasks VALUES ('task-old','repo-1','t','/tmp/wt','b','main','idle',1,1,NULL,NULL);
                ",
            )
            .unwrap();
        }

        // Opening migrates (adds ticket_key) without data loss.
        let store = TaskStore::open(&db_path).expect("open + migrate old db");
        assert_eq!(store.get_task("task-old").unwrap().unwrap().ticket_key, None);

        // A new task with a ticket_key round-trips on the migrated DB.
        let mut t = sample_task("task-new", "repo-1");
        t.ticket_key = Some("TST-9".to_string());
        store.insert_task(&t).unwrap();
        assert_eq!(store.get_task("task-new").unwrap().unwrap().ticket_key, Some("TST-9".to_string()));
    }

    #[test]
    fn reconcile_interrupted_setups_flips_running_to_failed() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();

        // Task 1: setup_status = Running (should be flipped to Failed)
        let mut t_running = sample_task("task-running", "repo-1");
        t_running.setup_status = Some(SetupStatus::Running);
        store.insert_task(&t_running).unwrap();

        // Task 2: setup_status = Succeeded (should be unchanged)
        let mut t_succeeded = sample_task("task-succeeded", "repo-1");
        t_succeeded.setup_status = Some(SetupStatus::Succeeded);
        store.insert_task(&t_succeeded).unwrap();

        // Task 3: setup_status = None (should be unchanged)
        let t_none = sample_task("task-none", "repo-1");
        store.insert_task(&t_none).unwrap();

        store.reconcile_interrupted_setups().unwrap();

        // Running must now be Failed.
        assert_eq!(
            store.get_task("task-running").unwrap().unwrap().setup_status,
            Some(SetupStatus::Failed)
        );
        // Succeeded must be unchanged.
        assert_eq!(
            store.get_task("task-succeeded").unwrap().unwrap().setup_status,
            Some(SetupStatus::Succeeded)
        );
        // None must be unchanged.
        assert_eq!(
            store.get_task("task-none").unwrap().unwrap().setup_status,
            None
        );
    }

    #[test]
    fn set_task_setup_status_round_trips_and_clears() {
        let (_dir, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        let mut task = sample_task("task-1", "repo-1");
        task.setup_status = Some(SetupStatus::Running);
        store.insert_task(&task).unwrap();

        // Inserted Running round-trips.
        assert_eq!(store.get_task("task-1").unwrap().unwrap().setup_status, Some(SetupStatus::Running));

        // Update to Succeeded.
        store.set_task_setup_status("task-1", Some(SetupStatus::Succeeded)).unwrap();
        assert_eq!(store.get_task("task-1").unwrap().unwrap().setup_status, Some(SetupStatus::Succeeded));

        // Clear to NULL.
        store.set_task_setup_status("task-1", None).unwrap();
        assert_eq!(store.get_task("task-1").unwrap().unwrap().setup_status, None);
    }

    #[test]
    fn hidden_column_migration() {
        let dir = TempDir::new().expect("create temp dir");
        let db_path = dir.path().join("test.db");
        let store = TaskStore::open(&db_path).expect("open + migrate");

        // Verify column exists
        let cols: Vec<String> = store
            .conn
            .prepare("PRAGMA table_info(tasks)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert!(cols.contains(&"hidden".to_string()));

        // Verify default is 0 (false) by inserting and retrieving
        let mut task = sample_task("task-1", "repo-1");
        task.hidden = false;
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_task(&task).unwrap();
        let retrieved = store.get_task("task-1").unwrap().unwrap();
        assert!(!retrieved.hidden);

        // Verify true value persists
        let mut task2 = sample_task("task-2", "repo-1");
        task2.hidden = true;
        store.insert_task(&task2).unwrap();
        let retrieved2 = store.get_task("task-2").unwrap().unwrap();
        assert!(retrieved2.hidden);
    }

    #[test]
    fn prompts_crud_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("t.db")).unwrap();

        assert!(store.list_prompts().unwrap().is_empty());

        store
            .insert_prompt(&Prompt { id: "a".into(), label: "Alpha".into(), body: "go alpha".into(), position: 0 })
            .unwrap();
        store
            .insert_prompt(&Prompt { id: "b".into(), label: "Beta".into(), body: "go beta".into(), position: 1 })
            .unwrap();

        let all = store.list_prompts().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].label, "Alpha");
        assert_eq!(all[1].body, "go beta");

        store.update_prompt("a", "Alpha2", "go alpha2").unwrap();
        let a = store.list_prompts().unwrap().into_iter().find(|p| p.id == "a").unwrap();
        assert_eq!(a.label, "Alpha2");
        assert_eq!(a.body, "go alpha2");

        store.delete_prompt("b").unwrap();
        let all = store.list_prompts().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, "a");
    }

    #[test]
    fn reorder_prompts_rewrites_positions() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("t.db")).unwrap();
        for (i, id) in ["a", "b", "c"].iter().enumerate() {
            store
                .insert_prompt(&Prompt { id: (*id).into(), label: (*id).into(), body: (*id).into(), position: i as i64 })
                .unwrap();
        }

        store.reorder_prompts(&["c".into(), "a".into(), "b".into()]).unwrap();

        let ordered: Vec<String> = store.list_prompts().unwrap().into_iter().map(|p| p.id).collect();
        assert_eq!(ordered, vec!["c", "a", "b"]);
    }

    #[test]
    fn task_status_pending_round_trips() {
        assert_eq!(TaskStatus::Pending.as_str(), "pending");
        assert_eq!(TaskStatus::from_str("pending").unwrap(), TaskStatus::Pending);
    }

    #[test]
    fn task_dependencies_edge_crud() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("v.sqlite3")).unwrap();
        let repo = sample_repo("r1");
        store.insert_repo(&repo).unwrap();
        for id in ["dep", "a", "b"] {
            let mut t = sample_task(id, "r1");
            t.status = TaskStatus::Pending;
            store.insert_task(&t).unwrap();
        }

        // Two tasks queued on "dep".
        store.add_task_dependency("a", "dep").unwrap();
        store.add_task_dependency("b", "dep").unwrap();
        let mut dependents = store.dependents_of("dep").unwrap();
        dependents.sort();
        assert_eq!(dependents, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(store.unmet_dependency_count("a").unwrap(), 1);

        // Removing edges → dep clears both.
        store.remove_dependencies_on("dep").unwrap();
        assert!(store.dependents_of("dep").unwrap().is_empty());
        assert_eq!(store.unmet_dependency_count("a").unwrap(), 0);
    }

    #[test]
    fn task_dependencies_many_edges_and_cascade() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("v.sqlite3")).unwrap();
        store.insert_repo(&sample_repo("r1")).unwrap();
        for id in ["d1", "d2", "waiter"] {
            let mut t = sample_task(id, "r1");
            t.status = TaskStatus::Pending;
            store.insert_task(&t).unwrap();
        }
        // A task waiting on TWO deps: proves the model is multi-dep ready.
        store.add_task_dependency("waiter", "d1").unwrap();
        store.add_task_dependency("waiter", "d2").unwrap();
        assert_eq!(store.unmet_dependency_count("waiter").unwrap(), 2);
        store.remove_dependencies_on("d1").unwrap();
        assert_eq!(store.unmet_dependency_count("waiter").unwrap(), 1); // still waiting on d2

        // Deleting the waiter task cascades its remaining edges away.
        store.delete_task("waiter").unwrap();
        assert!(store.dependents_of("d2").unwrap().is_empty());
    }

    // TASK-182: `promote_dependents_of` must capture its promotion set atomically
    // with removing the blocker's edges. A dependent edge inserted in the
    // queue-vs-finish window (a launch queues `waiter → blocker` while the blocker
    // still exists) must be BOTH returned for promotion AND cleared in the same
    // step — otherwise the edge is deleted with no promoter (or left dangling once
    // the blocker row is gone) and the waiter is stranded Pending forever.
    // `take_dependents_on` folds the read + remove into one transaction; this is
    // the seam the fixed promote path consumes.
    #[test]
    fn take_dependents_on_reads_and_removes_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("v.sqlite3")).unwrap();
        store.insert_repo(&sample_repo("r1")).unwrap();
        for id in ["blocker", "waiter"] {
            let mut t = sample_task(id, "r1");
            t.status = TaskStatus::Pending;
            store.insert_task(&t).unwrap();
        }
        // The waiter is queued behind the blocker (the in-window insert).
        store.add_task_dependency("waiter", "blocker").unwrap();

        // The returned set is the authoritative promotion list...
        let promote = store.take_dependents_on("blocker").unwrap();
        assert_eq!(promote, vec!["waiter".to_string()]);
        // ...and the edges are gone in the same step, so the waiter is promotable,
        // never stranded.
        assert!(store.dependents_of("blocker").unwrap().is_empty());
        assert_eq!(store.unmet_dependency_count("waiter").unwrap(), 0);
    }

    // No dependents → empty set, no error (the cheap early-exit case).
    #[test]
    fn take_dependents_on_empty_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("v.sqlite3")).unwrap();
        store.insert_repo(&sample_repo("r1")).unwrap();
        let mut t = sample_task("blocker", "r1");
        t.status = TaskStatus::Pending;
        store.insert_task(&t).unwrap();
        assert!(store.take_dependents_on("blocker").unwrap().is_empty());
    }

    #[test]
    fn blockers_of_returns_all_outstanding_blockers() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("v.sqlite3")).unwrap();
        store.insert_repo(&sample_repo("r1")).unwrap();
        for id in ["b1", "b2", "waiter"] {
            let mut t = sample_task(id, "r1");
            t.status = TaskStatus::Pending;
            store.insert_task(&t).unwrap();
        }
        store.add_task_dependency("waiter", "b1").unwrap();
        store.add_task_dependency("waiter", "b2").unwrap();

        let mut blockers = store.blockers_of("waiter").unwrap();
        blockers.sort();
        assert_eq!(blockers, vec!["b1".to_string(), "b2".to_string()]);
        // A task with no edges has no blockers.
        assert!(store.blockers_of("b1").unwrap().is_empty());
    }

    // TASK-177: a task queued behind TWO blockers stays not-ready until BOTH
    // blockers' edges are cleared. This is the exact predicate promote_dependents_of
    // gates on (unmet_dependency_count == 0). Landing a strict subset must not
    // make it ready.
    #[test]
    fn promotion_gate_requires_all_blockers_landed() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("v.sqlite3")).unwrap();
        store.insert_repo(&sample_repo("r1")).unwrap();
        for id in ["b1", "b2", "waiter"] {
            let mut t = sample_task(id, "r1");
            t.status = TaskStatus::Pending;
            store.insert_task(&t).unwrap();
        }
        store.add_task_dependency("waiter", "b1").unwrap();
        store.add_task_dependency("waiter", "b2").unwrap();

        // Nothing landed yet → not ready.
        assert_eq!(store.unmet_dependency_count("waiter").unwrap(), 2);

        // First blocker lands (its edges are removed) → strict subset, still NOT ready.
        store.remove_dependencies_on("b1").unwrap();
        assert_eq!(store.unmet_dependency_count("waiter").unwrap(), 1);

        // Last blocker lands → ready to promote exactly once.
        store.remove_dependencies_on("b2").unwrap();
        assert_eq!(store.unmet_dependency_count("waiter").unwrap(), 0);
    }

    #[test]
    fn promote_task_activates_pending_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = TaskStore::open(&dir.path().join("v.sqlite3")).unwrap();
        store.insert_repo(&sample_repo("r1")).unwrap();
        let mut t = sample_task("t1", "r1");
        t.status = TaskStatus::Pending;
        t.worktree_path = String::new();
        t.branch = String::new();
        t.pending_prompt = Some("seed".to_string());
        store.insert_task(&t).unwrap();

        store.promote_task("t1", "/wt/t1", "task-91-x").unwrap();
        let got = store.get_task("t1").unwrap().unwrap();
        assert_eq!(got.status, TaskStatus::Idle);
        assert_eq!(got.worktree_path, "/wt/t1");
        assert_eq!(got.branch, "task-91-x");
        assert_eq!(got.pending_prompt, None);
    }

    // ── Schedules (TASK-173) ──────────────────────────────────────────────

    fn sample_schedule(id: &str, repo_id: &str) -> Schedule {
        Schedule {
            id: id.to_string(),
            repo_id: repo_id.to_string(),
            name: "Weekly scan".to_string(),
            prompt: "/security-scan".to_string(),
            cron: "0 7 * * 1".to_string(),
            agent: None,
            model: None,
            base_branch: None,
            enabled: true,
            next_run_at: Some(1_000),
            last_run_at: None,
            created_at: 10,
            updated_at: 10,
            one_shot: false,
            skip_repo_prompt: true,
        }
    }

    #[test]
    fn schedule_insert_and_list_roundtrip() {
        let (_tmp, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_schedule(&sample_schedule("s1", "repo-1")).unwrap();

        let got = store.list_schedules("repo-1").unwrap();
        assert_eq!(got, vec![sample_schedule("s1", "repo-1")]);
    }

    #[test]
    fn schedule_cascades_on_repo_delete() {
        let (_tmp, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_schedule(&sample_schedule("s1", "repo-1")).unwrap();

        store.delete_repo("repo-1").unwrap();

        assert!(store.get_schedule("s1").unwrap().is_none());
    }

    #[test]
    fn due_schedules_filters_disabled_future_and_null() {
        let (_tmp, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();

        let mut due = sample_schedule("due", "repo-1");
        due.next_run_at = Some(500);
        let mut future = sample_schedule("future", "repo-1");
        future.next_run_at = Some(2_000);
        let mut disabled = sample_schedule("disabled", "repo-1");
        disabled.enabled = false;
        disabled.next_run_at = Some(100);
        let mut never = sample_schedule("never", "repo-1");
        never.next_run_at = None;
        for s in [&due, &future, &disabled, &never] {
            store.insert_schedule(s).unwrap();
        }

        let got = store.due_schedules(1_000).unwrap();
        let ids: Vec<&str> = got.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["due"]);
    }

    #[test]
    fn advance_schedule_updates_next_and_last() {
        let (_tmp, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        store.insert_schedule(&sample_schedule("s1", "repo-1")).unwrap();

        store.advance_schedule("s1", Some(9_999), 1_000, 1_000).unwrap();

        let got = store.get_schedule("s1").unwrap().unwrap();
        assert_eq!(got.next_run_at, Some(9_999));
        assert_eq!(got.last_run_at, Some(1_000));
    }

    #[test]
    fn one_shot_schedule_roundtrips() {
        let (_tmp, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        let mut s = sample_schedule("once", "repo-1");
        s.one_shot = true;
        s.cron = String::new();
        s.next_run_at = Some(5_000);
        store.insert_schedule(&s).unwrap();

        let got = store.get_schedule("once").unwrap().unwrap();
        assert!(got.one_shot);
        assert_eq!(got.cron, "");
        assert_eq!(got.next_run_at, Some(5_000));
    }

    #[test]
    fn skip_repo_prompt_roundtrips_and_updates() {
        // TASK-181: the per-schedule skip flag persists through insert/get and
        // through update_schedule_fields (both true→false and false→true).
        let (_tmp, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();

        // Default fixture is skip = true; a schedule can also be stored as false.
        let mut including = sample_schedule("include", "repo-1");
        including.skip_repo_prompt = false;
        store.insert_schedule(&including).unwrap();
        store.insert_schedule(&sample_schedule("skip", "repo-1")).unwrap();

        assert!(!store.get_schedule("include").unwrap().unwrap().skip_repo_prompt);
        assert!(store.get_schedule("skip").unwrap().unwrap().skip_repo_prompt);

        // update_schedule_fields carries the flag through a full-replace edit.
        store
            .update_schedule_fields(
                "include", "Weekly scan", "/security-scan", "0 7 * * 1",
                None, None, None, true, /* skip_repo_prompt */ true, Some(1_000), 20,
            )
            .unwrap();
        assert!(store.get_schedule("include").unwrap().unwrap().skip_repo_prompt);
    }

    #[test]
    fn retire_schedule_disables_and_clears_next() {
        let (_tmp, store) = open_test_store();
        store.insert_repo(&sample_repo("repo-1")).unwrap();
        let mut s = sample_schedule("once", "repo-1");
        s.one_shot = true;
        s.next_run_at = Some(500);
        store.insert_schedule(&s).unwrap();

        store.retire_schedule("once", 1_234, 1_234).unwrap();

        let got = store.get_schedule("once").unwrap().unwrap();
        assert!(!got.enabled);
        assert_eq!(got.next_run_at, None);
        assert_eq!(got.last_run_at, Some(1_234));
        // A retired one-shot is never re-selected as due.
        assert!(store.due_schedules(9_999).unwrap().is_empty());
    }

    #[test]
    fn in_place_columns_round_trip_and_default_false() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("v.db");
        let store = TaskStore::open(&db).unwrap();

        let repo = Repo {
            id: "r1".into(),
            name: "r".into(),
            path: "/tmp/r".into(),
            default_branch: "main".into(),
            remote_url: None,
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
        };
        store.insert_repo(&repo).unwrap();
        assert!(!store.get_repo("r1").unwrap().unwrap().in_place_default);

        store.set_repo_in_place_default("r1", true).unwrap();
        assert!(store.get_repo("r1").unwrap().unwrap().in_place_default);

        let task = Task {
            id: "t1".into(),
            repo_id: "r1".into(),
            title: "t".into(),
            worktree_path: "/tmp/r".into(),
            branch: "main".into(),
            base_branch: "main".into(),
            status: TaskStatus::Idle,
            created_at: 0,
            updated_at: 0,
            pr_number: None,
            pr_url: None,
            ticket_key: None,
            agent: None,
            model: None,
            setup_status: None,
            hidden: false,
            pending_prompt: None,
            auto_approve: None,
            in_place: true,
        };
        store.insert_task(&task).unwrap();
        assert!(store.get_task("t1").unwrap().unwrap().in_place);

        // Idempotent migration: re-opening the same DB must not panic and keeps values.
        drop(store);
        let store2 = TaskStore::open(&db).unwrap();
        assert!(store2.get_task("t1").unwrap().unwrap().in_place);
    }
}
