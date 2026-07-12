# Agents and the terminal

Every task has an embedded terminal running a real PTY, with an agent (or a plain shell)
attached to it. This page covers picking an engine and model, starting/stopping/resuming an
agent, opening extra shell tabs, and sending it a launch prompt — plus the keep-alive
guarantee that keeps a session's terminal (and process) alive while you switch between tasks.

## What it is

Each task's center pane is a real terminal: a PTY (pseudo-terminal) spawned in the task's
worktree, with its output streamed live to an [xterm.js](https://xtermjs.org/) view and its
keystrokes written straight back to the process. The first tab is the **agent** — whichever
engine the task is configured to run — and you can open additional plain-shell tabs alongside
it for running tests, `git`, or anything else, all still scoped to the task's worktree.

The agent itself is pluggable: La Vigie ships several built-in engines (Claude Code, Aider,
Codex, Antigravity, Cursor, OpenCode, Mistral Vibe) and lets you register your own **custom
agent** pointing at any CLI. Claude Code gets rich status (working / idle / needs attention)
via Claude Code hooks; every other engine — built-in or custom — reports only whether its
process is running.

## How to use it

**Choose an engine and model.** Before starting a task's agent, the agent/model picker next to
**Start agent** lets you choose which engine to run and (for engines that advertise one) which
model. The picker's precedence is task override → repo default → the app's global default
(Claude Code). Only OpenCode advertises models today — picking it reveals a model submenu
populated by shelling out to the engine's own `models` listing; other engines show no model
submenu and launch with their own default. Registering and removing custom agents is done from
the app's Settings.

**Set auto-approve mode.** Some engines can run without pausing for per-action confirmation —
Mistral Vibe's `--auto-approve`. Rather than hardcoding that on, you control it with a tri-state
setting: a per-repo default in **Repo Settings → "Auto-approve agent actions"** (Use default /
On / Off) and a per-task override in both the **New Task** dialog and **Task Detail** (Inherit
from repo / On / Off). The effective value resolves task override → repo default → on (preserving
the historical always-on default), and when it's on the engine's auto-approve flag is added at
launch and the session shows an "auto" indicator. Engines with no auto-approve flag (e.g. Claude
Code) ignore the setting.

**Start / resume / stop.** With no agent running, the terminal pane shows **Start agent** and
**Resume**. Resume is only enabled for engines that support it (Claude Code and Mistral Vibe
resume with `--continue`; the rest have no resume path and launch fresh every time). Once
running, a **Stop** action kills the process and tears down its PTY.

**Open extra shell tabs.** The **+** tab next to the agent tab opens a plain login shell in the
same worktree — no hooks, no agent status, just your normal shell environment. Close a shell
tab with its **×**; the agent tab itself isn't closable this way, only stoppable.

**Send a launch prompt.** The task-creation dialog has a prompt field you can type into
directly, or fill from the **Library** dropdown, which lists your saved prompts. Once an
agent is running, the same **Library** picker sits above the terminal so you can drop a saved
prompt (or a diff comment turned into a prompt) straight into the running session. Manage the
library itself — add, edit, delete, and reorder saved prompts — from **Manage prompts…** in
that same dropdown.

## How it works

Each session is a real OS process spawned via [`portable-pty`](https://docs.rs/portable-pty) —
either the resolved agent binary (with the flags for the chosen engine, model, and hook/MCP
wiring) or the user's login shell for a `shell` tab. A background thread reads the PTY's raw
output and streams it to the frontend over a Tauri `Channel` as base64-encoded chunks; keystrokes
in xterm.js are written straight back to the process's stdin. Every La Vigie-spawned process is
tagged with `LAVIGIE=1` in its environment so cooperating tools can detect they're running inside
the app.

Only the `claude` engine gets the hook pipeline: at launch it's handed `--settings` with inline
hook definitions (and `--mcp-config` registering La Vigie's own MCP server) so Claude Code POSTs
status callbacks to a local bridge instead of the app scraping terminal text for status. Every
other engine — built-in or custom — is "lifecycle-only": its status is just whether the process
is still alive.

**Keep-alive.** The terminal for a session must never unmount while its agent is running —
unmounting an xterm instance tears down the component that owns the PTY connection, killing the
underlying process. To guarantee that, one `TerminalHost` renders a persistent view for *every*
session across *every* task, keyed by `taskId:sessionId`, and hides all but the active one with
`display: none` rather than conditionally rendering only the selected task's session. Switching
tasks, toggling tabs, or resizing panels only moves what's shown around this host — it never
remounts it.

**Sizing.** A PTY is spawned at a fixed 80×24 and then re-fit to its actual container size right
after the process starts, whenever the terminal's container is resized (via `ResizeObserver`),
and on OS window resize (`ResizeObserver` alone doesn't reliably catch that). Each re-fit both
resizes the xterm view and tells the backend PTY its new column/row count, so the process's own
notion of terminal size stays correct.

## Extending it

**Add a custom agent engine.** A custom agent is just a name, a display name, a binary to
resolve, and argv fragments — base args, resume args (empty if the engine can't resume), extra
args, and optionally a model flag plus the argv to list available models. Custom agents are
always lifecycle-only (no hook status) and can't reuse a built-in engine's name; registering one
makes it selectable everywhere the built-ins are, including per-task and per-repo default
selection.

**Per-repo and per-task model/engine defaults.** Selection resolves task override → repo default
→ global default, so you can set a repo-wide default engine or model once and only override it
on individual tasks that need something different.

**Auto-approve per engine.** Each engine advertises its own auto-approve flag(s) via
`auto_approve_args` on its spec (`src-tauri/src/agent/spec.rs`) — Mistral Vibe supplies
`--auto-approve`, while an engine that leaves it empty simply ignores the setting. The effective
on/off resolves in `effective_auto_approve` (task override → repo default → on) and the flag is
appended in `build_agent_command` at launch.
