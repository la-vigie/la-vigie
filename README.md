# La Vigie

A local desktop app to manage parallel **AI coding agents** and their **git worktrees** from one
window — a local alternative to hosted "cloud agent" dashboards, centered on the code/diff. Instead
of juggling 20–30 tmux windows, each **task** is a git worktree + branch: you run an agent inside it
in an embedded terminal, watch live status, review the diff, open/merge PRs, and clean up — all in
one place.

Built with **Tauri 2** (Rust) + **React/TypeScript**.

## Features

- **Guided first-run tour:** on first launch, an interactive product tour overlays coach-marks on
  the real UI — walking you through the core loop (add a repo → create a task → start an agent →
  watch it work → review the diff), then schedules/automation and remote/mobile pairing. It's
  skippable, resumable, and re-launchable anytime from the **?** button in the title bar.
- Multi-repo sidebar; create tasks that each get their own git worktree + branch.
- **Work-in-place tasks (no worktree):** for tiny single-folder projects that must stay in one
  folder with all their context, opt a task out of worktree isolation so its agent runs directly in
  the repo's existing checkout. Toggle per task ("Work in place — no worktree" in the New-Task form)
  or set a repo default in Repository settings. Optionally names a new branch to `checkout -b` in the
  folder, else works on the current branch; only one in-place task per repo, and teardown always
  preserves the folder and branch.
- Embedded terminal per agent (spawned via a real PTY) with start / stop / resume. Pluggable agent
  engines (e.g. Claude Code and others).
- **Structured chat surface over ACP:** pick an ACP engine (`Claude Code (ACP)` or
  `Mistral Vibe (ACP)`) in the agent picker and the task runs over the
  [Agent Client Protocol](https://agentclientprotocol.com) instead of a terminal — a conversation
  timeline with message bubbles, collapsible thoughts, tool-call cards (with diffs), the agent's
  plan, a reply composer with cancel, in-app permission prompts, a session-mode switcher, and
  context/cost usage. Stop a session and **Resume** it later: the backend reconnects the agent and
  re-establishes the prior conversation (ACP `session/load`, falling back to `session/resume` or a
  replay of the app's own event log), and the timeline is restored. PTY and ACP engines coexist;
  shells and other engines keep the terminal.
- **Usage & cost panel:** the **📊** button in the title bar opens a per-model spend view —
  estimated cost, peak context, and rate-limit status over a chosen window (24h / 7 / 30 / 90 days),
  broken down by provider/model and by task. It answers "what did this week of agents cost, by
  model?". Covers ACP-engine sessions, which report usage and cost natively (the PTY path emits no
  usage); cost comes straight from the agent's own figure, shown only when the agent reports one.
- **Per-task model selection:** pick the model an engine runs (and set a repo default) from the
  agent picker in the New-Task form and Repository settings. Engines that can enumerate their models
  (e.g. OpenCode) show a list; Claude Code — which has no `models` command — offers free-text entry
  with quick-pick tier aliases (`opus` / `sonnet` / `haiku`). Left unset, the engine uses its own
  default (no `--model` flag passed).
- **Auto-routing (per-repo policy):** give a repo a JSON routing policy and new tasks created
  without an explicit agent are dispatched to an engine (and model) automatically — from a quick
  difficulty/kind classification of the task and/or a case-insensitive title match, with a fallback
  and first-match-wins rules. Edit it in Repository settings (with a Format button and live
  validation); the New-Task form shows an "Auto-route" toggle, and the chosen engine and reason
  appear on the task. A per-task agent pick always overrides the policy.
- Live, at-a-glance status driven by agent hooks (working / needs-attention / idle / error) + OS
  notifications. When a turn ends in failure, the task header shows a dismissible **error banner**
  explaining why (the reason is also included in the notification body), so an error state is never
  just a silent red dot.
- **System-tray menu:** a menu-bar item lists your in-progress tasks grouped by repo, each with a
  status glyph, ticket key, and title — so you can jump straight back to any active task even when the
  window is hidden. Picking a task brings the window to the front and selects it; the menu updates live
  as tasks start, change status, and finish.
- Local diff review (GitHub-style, syntax-highlighted) with stage & commit.
- GitHub PR integration via the `gh` CLI: create PR, status/checks, read-only comments, squash-merge.
- Optional bundled "way of working" skills: an off-by-default Settings toggle injects La Vigie's own
  Claude Code skills (`/lavigie:rename`, `/lavigie:finished`, `/lavigie:spec-init`,
  `/lavigie:verify-claims`, `/lavigie:await-merge`, `/lavigie:dispatch-backlog`) into launched agents
  — namespaced, so they add to rather than override your own `~/.claude` skills.
- **Task dependencies ([start-on-merge](./docs/start-on-merge.md)):** queue a task behind one or more
  others and have it auto-start once they all land.
- **Recurring schedules:** give a repo a cron schedule that auto-launches a task from a stored prompt
  (e.g. a `/security-scan` skill) when it's due — managed in Repository settings → Schedules, with a
  live next-run preview. If the app was closed at the scheduled time, it catches up once on next launch.
- **One-time (deferred) launch:** schedule a task to launch *once* at a future time — "in N hours" —
  then it retires. Create one from the New-Task form ("Start later") or Repository settings →
  Schedules (Recurring | One-time), or let an agent defer its own via the `schedule_task` MCP tool.
  Handy for kicking off work when your Claude quota resets; it catches up once if the app was closed
  at that time.
- Collapsible sidebar, resizable panes, diff right-or-bottom.

## Prerequisites

- **Node.js** (18+) and npm
- **Rust** via [rustup](https://rustup.rs/) (stable toolchain)
- **Xcode Command Line Tools** (macOS) — `xcode-select --install`
- **[`gh` CLI](https://cli.github.com/)**, authenticated (`gh auth login`), for the PR features
- A supported AI coding agent CLI on your `PATH` for the agent terminal

See the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for full platform details.

## Build & run

```bash
npm install
npm run tauri dev              # run the app in dev (Vite dev server + Tauri window; first build is slow)
npm run tauri:build            # release bundle, signed with your Apple Development cert (macOS)
npm run tauri:build:unsigned   # release bundle, ad-hoc signed — no Apple certificate needed
```

On macOS the default `tauri:build` signs with an **Apple Development** certificate, so macOS
privacy grants (Screen Recording, …) survive rebuilds. **No certificate? Use
`tauri:build:unsigned`** — the default will fail to sign without one. See
[dev-signing](./docs/dev-signing.md) for both options and their trade-offs.

Or run `./scripts/setup-worktree.sh` (also `npm run setup`) to install JS deps and warm the Rust
build in one step.

## Getting started

New to La Vigie? The [Getting Started guide](./docs/getting-started.md) walks through the core
loop — creating a task, running an agent, reviewing the diff, and steering the agent — with
screenshots of each step. For a page on every feature, browse the full
[documentation index](./docs/README.md).

## Testing

```bash
npx vitest run                 # frontend tests
cd src-tauri && cargo test     # Rust tests
npm run build                  # frontend typecheck + build
```

## Contributing

Contributions are welcome! See [CONTRIBUTING.md](./CONTRIBUTING.md) for the architecture overview,
project conventions, and the dev/test workflow, and [CODE_OF_CONDUCT.md](./CODE_OF_CONDUCT.md) for
community expectations. Security issues: please read [SECURITY.md](./SECURITY.md).

## License

Licensed under the [MIT License](./LICENSE).
