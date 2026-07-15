# Remote control

## What it is

Remote control lets you reach your La Vigie cockpit from another device — typically your
phone — over your [Tailscale](https://tailscale.com) tailnet. Enable it from the Mac and
you get a small mobile-friendly web client that can browse your repositories and tasks,
read a running agent's conversation, reply to it, and kick off a worktree-less session for
quick asks: either the legacy global "concierge" (read-only across every repo) or a
per-repo "orchestrator" session scoped to one repo, which can also dispatch/finish/
schedule tasks there. It's off by default and never reachable over the public internet —
only devices on your tailnet, holding a pairing token, can talk to it.

## How to use it

**Enable it.** Open **Settings → Remote access** and click **Enable remote**. La Vigie
mints a pairing token, starts a local server, and fronts it with `tailscale serve` so it's
reachable at your Mac's tailnet address over HTTPS. The panel shows the pairing token, the
URL to open on your phone, and a **QR code** that encodes both — scan it with your phone's
camera and it opens the remote client already paired, no typing required. The token lives
only in the link's fragment (after `#`), so it never reaches the server or its logs — but
anyone who can see the screen (or a screenshot of it) can pair, so keep it private and
disable remote control when you're done.

If you'd rather not scan a QR code, open the URL manually and paste the pairing token into
the "Pairing token" field on the client's connect screen.

**Browse and create tasks.** The client opens on a list of your repositories; tapping one
shows its tasks with a live status dot, ticket chip, and PR badge, mirroring the desktop
sidebar. A **+ New** button lets you create a task from your phone (title, optional ticket
key, optional initial prompt) — it starts the same way a task created from the desktop New
task dialog would.

**Read and reply to a session.** Opening a task shows its live conversation, polling for
new messages, with a reply box at the bottom. Sending a reply delivers it to the task's
running agent exactly as if you'd typed it into the embedded terminal.

**Concierge sessions.** For a quick ask that doesn't need its own worktree, the client can
start a singleton "concierge" Claude session — a rootless conversation not tied to any
task or repo. It's addressed through the same read/reply UI as a task. A concierge session
auto-resumes across restarts and is reaped automatically after 15 minutes of inactivity.
The concierge's MCP token is read-only (cross-repo `list_tasks`/`task_status`/
`get_task_activity`/`list_repos` only — see [MCP server](mcp-server.md)).

**Orchestrator sessions.** For repo-scoped work — dispatching, finishing, or scheduling
tasks in one specific repo without occupying a task agent's context — the client can open
a per-repo "orchestrator" session, keyed `orchestrator:{repoId}`. It's the same
worktree-less conversation the desktop app's pinned per-repo "Orchestrator" sidebar row
opens (TASK-126): there's one live session per repo, resumed rather than duplicated no
matter which surface opens it, so a task you dispatch from your phone shows up if you
later open the same repo's orchestrator on the Mac. It's addressed through the same
read/reply UI as a task, auto-resumes, and is
reaped after 15 minutes of inactivity like the concierge, but its MCP token is scoped
`Orchestrator{repoId}` — it can act (start/finish/schedule tasks), confined to that repo.

**Schedules.** From a repo's task list, tap **Schedules** to see that repo's recurring and
one-time schedules — each with its cron (or "One-time"), next-run time, and enabled state,
mirroring the desktop Repository settings → Schedules tab. You can flip a schedule on/off,
delete it, and create a new one (recurring by cron, or one-time in N hours). Changes round-trip
to the same store the desktop reads, so the two stay in sync.

Back on the Mac, **Settings → Remote access → Remote sessions** lists any running remote
sessions (concierge and any open orchestrators, labeled `orchestrator · <repo name>`) with
their idle time and a **Stop** button. **Settings → Remote access → Orchestrators** also
lists an **Open orchestrator** button per repo, for opening one without a phone.

**Sleep.** While remote control is active, La Vigie holds a system power assertion so the
Mac doesn't idle-sleep and become unreachable. The Settings panel tells you whether that
assertion was acquired; it's best-effort and only honored on AC power, so on battery
(especially with the lid closed) the Mac may still sleep regardless.

**Disable it.** Click **Disable remote** to tear the server down, reset the `tailscale
serve` mapping, and invalidate the pairing token — anyone still holding the old token gets
rejected immediately.

## How it works

Remote control is a small [axum](https://github.com/tokio-rs/axum) HTTP server bound to an
ephemeral loopback port and fronted by `tailscale serve --bg --https 443 …`, which gives it
automatic TLS on your Mac's MagicDNS name. Enabling refuses to proceed if Tailscale Funnel
(public internet exposure) is active on the machine, so the server is always tailnet-only.

Every request except `GET /` is guarded by two checks: the `Host` header must match the
tailnet MagicDNS name (an anti DNS-rebinding check), and the `Authorization: Bearer <token>`
header must match the active pairing token via a constant-time comparison. `GET /` (the
static client page) only needs the Host check, since the page itself carries no secret.
Disabling remote drops the in-memory token, so every subsequent request 401s.

The server exposes this JSON API, all served under the enabled remote's tailnet URL:

| Route | Method | Purpose |
|---|---|---|
| `/` | GET | The static mobile client page |
| `/api/state` | GET | A snapshot of repos + tasks, same shape the desktop sidebar renders from |
| `/api/tasks` | POST | Create a task (`repoId`, `title`, optional `ticketKey`/`prompt`) — same launch path as the desktop New task dialog |
| `/api/concierge` | POST | Ensure a live concierge session exists (idempotent create-or-resume) |
| `/api/orchestrator/{repoId}` | POST | Ensure a live per-repo orchestrator session exists for `repoId` (idempotent create-or-resume); returns its `orchestrator:{repoId}` session id |
| `/api/tasks/{id}/session` | GET | Incremental read of a task's (or the concierge's/an orchestrator's) conversation, `?since=<byteOffset>` |
| `/api/tasks/{id}/reply` | POST | Deliver a plain-text reply to a task's (or the concierge's/an orchestrator's) live agent |
| `/api/repos/{id}/schedules` | GET | List a repo's schedules (recurring + one-shot) with `nextRunAt`, `enabled`, `oneShot` — same rows as the desktop Schedules tab |
| `/api/repos/{id}/schedules` | POST | Create a schedule: a `cron` ⇒ recurring, or `inSeconds`/`atUnix` ⇒ one-time (plus `name`, `prompt`) — reuses the same store CRUD + validation as the desktop create form |
| `/api/schedules/{id}/enabled` | POST | Arm/disarm a schedule (`{"enabled": bool}`); returns the updated schedule |
| `/api/schedules/{id}` | DELETE | Delete a schedule |

A reply is delivered to the agent's PTY as a bracketed paste followed by a separate `\r`
keystroke a moment later — sending them as one write can make the agent's TUI swallow the
Enter as part of the paste, leaving the text sitting unsubmitted.

The concierge session is a single rootless `claude` process spawned in a neutral working
directory rather than a task's worktree, wired with the same HookBridge hooks a normal
task agent gets (so its transcript is captured for the session-read route) and a
read-only, cross-repo `Concierge` MCP token (see [MCP server](mcp-server.md)). It's a
singleton: starting it twice resumes the same session rather than stacking processes, and
a background reaper stops it after 15 minutes of no read/reply activity. Because it has no
database row, it never shows up in the desktop task list — only in the Settings "Remote
sessions" panel.

A per-repo orchestrator session works the same way — rootless, no worktree, HookBridge-wired
— except it's keyed per repo (`orchestrator:{repoId}`) rather than a singleton, and its MCP
token is scoped `Orchestrator{repoId}` (TASK-180): it can act — `start_task`/`finish_task`/
`schedule_task`/schedule management — but only for that one repo. `ensure_orchestrator` (the
shared, liveness-gated glue behind both the `/api/orchestrator/{repoId}` route and the
desktop Settings **Open orchestrator** button) resumes the existing live session for that
repo if there is one, so the phone and the Mac always end up talking to the same
conversation. The desktop's pinned sidebar "Orchestrator" row (TASK-126) goes through a
sibling path, `open_orchestrator_terminal`, that additionally binds the session to a PTY
channel for terminal rendering — if a headless (phone-started) session is already live it
is stopped and respawned `--continue`, so the conversation resumes but the channel is now
the desktop terminal's. Like the concierge, it has no database row and is reaped after 15
minutes idle; it shows up in Settings "Remote sessions" as `orchestrator · <repo name>`.

## Extending it

The JSON API above is the integration surface: anything that can hold a Tailscale-reachable
bearer token can drive La Vigie the same way the bundled mobile client does — poll
`/api/state` for the task list, `POST /api/tasks` to launch one, and read/reply against
`/api/tasks/{id}/session` and `/api/tasks/{id}/reply` (or `/api/concierge` for a
worktree-less, cross-repo read session, or `/api/orchestrator/{repoId}` for a worktree-less
session scoped to and able to act on that one repo) to hold a conversation with an agent.
