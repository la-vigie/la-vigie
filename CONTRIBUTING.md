# Contributing to La Vigie

Thanks for your interest in contributing! This document covers how to build the project, the
architecture, and the conventions that keep the codebase consistent.

## Development setup

```bash
npm install
npm run tauri dev        # Vite dev server on :1420 + Tauri window (first build is slow)
```

Prerequisites: Node.js, Rust (via [rustup](https://rustup.rs/)), Xcode Command Line Tools (macOS),
and the [`gh` CLI](https://cli.github.com/) (authenticated) for the PR features.

### Tests & checks

```bash
npx vitest run                 # frontend tests
cd src-tauri && cargo test     # Rust tests
npm run build                  # frontend typecheck + build
cd src-tauri && cargo build    # Rust build — keep it warning-free
```

Please keep `cargo build` and test output warning-free.

> **Note:** some behavior can only be verified by running the app — the embedded terminal rendering,
> live hook-driven status dots, the folder picker, notifications, and the PR flow. Automated tests
> and builds are the gate for logic, but please run `npm run tauri dev` and confirm anything
> visual/runtime before claiming it works.

## Architecture

**Backend (`src-tauri/src/`)** — small, single-responsibility units; `commands.rs` is the Tauri glue:

- `store/` — SQLite (rusqlite) persistence: repos and tasks, CRUD, schema migration.
- `git/` — `git` CLI wrapper (worktree add/remove, diff vs base, status, stage, commit, branch delete).
- `agent/` — spawns an agent process in a worktree via a PTY, streams output over a Tauri channel,
  and handles write/resize/stop. Holds the live PTY registry in app state.
- `lavigie_plugin.rs` / `lavigie_skills.rs` — resolve La Vigie's bundled "way of working" skills and
  inject them into launched agents when the Settings toggle is on (per-engine, via the `AgentSpec`
  `skill_injection` strategy). `claude` gets the skill *plugin* (`resources/lavigie-plugin/`, a Claude
  Code plugin injected via `--plugin-dir`); the other engines — Codex, Antigravity, OpenCode, Mistral
  Vibe — get a per-provider bundle (`resources/lavigie-skills/<provider>/`) materialized into the
  worktree at spawn by `agent/skill_bundle.rs` and git-excluded so it stays out of the Diff. Both are
  **generated** from one canonical source — do not hand-edit `resources/lavigie-plugin/skills/` or
  `resources/lavigie-skills/`. *Extend it:* author the skill once in `.rulesync/skills/<name>/SKILL.md`,
  then run `npm run skills:generate` (rulesync) to regenerate every provider bundle + the Claude plugin,
  and commit the result. A CI drift guard (`.github/workflows/skills-drift.yml`) fails the PR if the
  committed bundles are stale. Claude-only frontmatter (e.g. `allowed-tools`, `argument-hint`) goes in a
  `claudecode:` block in the source.
- `hooks/` — a loopback HTTP server (ephemeral port) that receives agent hook callbacks and emits
  `agent_status` Tauri events.
- `github/` — `gh` CLI wrapper: PR status/comments/create/merge; pure JSON parsers unit-tested
  against fixtures.
- `state.rs` — shared app state (store, worktrees root, agent registry, hook port, resolved paths).

**Frontend (`src/`)** — React + Zustand; `api.ts` wraps the Tauri `invoke`/`Channel` bridge:

- `components/`: `Sidebar` (repos + tasks), `TaskDetail` (agent controls, finish flow, split
  terminal/review), `Terminal` (xterm.js host), `Review` (Diff | PR tabs), `Diff`, `Pr`, `StatusDot`.
- `hooks/useAgentStatus.ts` — listens to `agent_status` events → store + OS notifications.

## Conventions (these bite if violated)

- **Terminal keep-alive:** the terminal host must never unmount while an agent runs (it kills the
  PTY). Layout changes swap content *around* it, never wrap/remount it. Every layout change here
  needs a DOM-identity test.
- **Status is out-of-band:** agent status comes from agent hooks, never from scraping terminal output.
- **Rust locking:** never hold the store `Mutex` across an `.await`. Pattern: lock → capture values →
  drop the guard → then `await` git/gh work.
- **Errors:** map command errors with `format!("{e:#}")` so the underlying `git`/`gh` stderr reaches
  the UI (a plain `.to_string()` hides it).
- **GitHub only via the `gh` CLI** (no token storage/OAuth). All `gh`/`git` calls use argv vectors
  (no shell), so they're safe from injection via task titles/PR bodies. PR content is rendered as
  React text (never `dangerouslySetInnerHTML`).
- **Frontend↔Rust types** cross IPC as camelCase (`#[serde(rename_all = "camelCase")]`); keep them
  in sync.

## Testing conventions

- **Pure functions are the testable core**: Rust parsers/helpers (JSON parsers vs fixtures,
  slug/status helpers, DB migration) are unit-tested. Thin glue (Tauri command handlers,
  `gh`/`git`-invoking wrappers) is not unit-tested — it needs an app/network; say so rather than
  faking it.
- **Frontend**: Vitest with `invoke`/`Channel`/`@xterm/xterm`/plugins mocked. Assert real behavior
  (decoded bytes, DOM-node identity for keep-alive, command args), not mock construction.

## Keeping docs current (the doc-scan skill)

This repo ships a repo-local skill, `.claude/skills/scan-for-doc-change/`, that the `/ship` flow
runs automatically: when a branch adds user-facing surface — a new Tauri/IPC command, MCP tool,
HookBridge route, setting, agent engine, or UI feature — it emits an advisory and a drafted doc
entry so the change doesn't ship undocumented. It's advisory; it never blocks a merge, and it stays
silent for internal-only diffs (refactors, tests, CI).

*Extend it:* the detection heuristics and doc targets all live in that one `SKILL.md`. Teach it a
new user-facing surface kind, or a new doc target, by editing that file.

## Pull requests

1. Fork and branch from `main`.
2. Make your change with tests; keep builds and tests green and warning-free.
3. Open a PR describing the change and how you verified it (including any manual GUI check).

By contributing, you agree that your contributions are licensed under the [MIT License](./LICENSE).
