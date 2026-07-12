# Agent self-management

## What it is

Most of what La Vigie's backend does for a task is triggered from the UI — you click
Finish, you rename a task in the sidebar. But an agent can also drive its own task
directly, without any of that: it calls back into **HookBridge**, the same local
loopback server that receives Claude Code's status hooks (see
[Status and notifications](./status-and-notifications.md)), and asks for its own task
to be renamed or torn down. No polling, no UI round-trip — the agent just posts to a
URL it was handed on spawn.

## How to use it

La Vigie spawns every agent with three environment variables:

- **`LAVIGIE_HOOK_PORT`** — the port HookBridge is listening on (loopback only).
- **`LAVIGIE_TASK_ID`** — the durable id of the task the agent is running in. This is
  the key the two self-management endpoints below are keyed on, not the agent/PTY id —
  a task row survives an app restart, so a callback that presents `LAVIGIE_TASK_ID`
  still resolves correctly even if La Vigie's in-memory state has since been rebuilt.
- **`LAVIGIE_AGENT_ID`** — the id of this particular run, used for the per-run hook,
  status-line, and transcript endpoints.

An agent (or a skill running inside one) uses these to call two routes:

```
POST /rename/{task_id}
Body: the new title, as plain text (not JSON)

200  -> applied name (whitespace collapsed, trimmed, capped at 200 chars)
400  -> "task name is empty after trimming" (blank/whitespace-only body)
404  -> "unknown task" (task_id doesn't exist)
```

```
POST /finish/{task_id}?force=<bool>
Body: none

200  -> "finished"       (accepted — teardown will run to completion)
404  -> "unknown task"   (task_id doesn't exist)
409  -> <reason>          (unsafe to tear down; see safety gate below)
500  -> <error message>  (a git or database operation failed)
```

`/finish` is what the built-in `/finished` skill uses to wrap up a session: when it
detects the `LAVIGIE_*` variables in its environment, it POSTs to `/finish/{task_id}`
as its last action instead of the generic tmux-window teardown (`dwt` + closing the
window) it falls back to outside La Vigie.

## How it works

**Rename** is the simple case: `sink.set_task_name` looks the task up by
`task_id`, and only writes and returns `200` if it exists — an unknown id is a real
`404`, not a silent no-op, so a `200` genuinely means "renamed." A successful rename
also emits a `task_renamed` event so the sidebar and header update live.

**Finish** does real destructive work, so it runs through a shared two-phase core
(`prepare_teardown` / `perform_teardown`) rather than doing everything inline:

1. **Prepare** (cancellation-safe — mutates nothing) looks the task up by `task_id`,
   and — unless `force=true` — runs a safety gate: refuse if the worktree has any
   uncommitted or untracked changes, and refuse if the branch has commits that aren't
   merged to its base (checked against a freshly-fetched `origin/<base>`, falling back
   to the local branch if there's no remote to fetch). `force=true` skips this gate
   entirely. Either refusal comes back as a `409` with the reason as the body; a
   passing task moves on to phase 2.
2. **Perform** (must run to completion once started) stops the task's live agent PTY,
   removes the git worktree, then deletes the task's database row. The branch itself
   is never deleted here — it's kept, so committed work (merged or not, once it's past
   the gate) survives the teardown.

The tricky part is *how* perform runs. Stopping the PTY in step 2 kills the very
process that made the `/finish` request — if the HTTP handler awaited the teardown
directly, its own request future could be torn down mid-sequence by that same PTY
dying, leaving the worktree half-removed. So the handler only awaits the safety-gate
phase; once a plan is ready, it spawns phase 2 **detached** and returns `200`
immediately. That means a `200` from `/finish` means "accepted, teardown is running to
completion in the background" — not that the worktree is already gone. When that
detached phase finishes, it emits a `task_removed` event (with the `task_id`) so the
sidebar drops the task live, since self-teardown never went through the webview
round-trip that would otherwise trigger a refresh.

## Extending it

Rename and finish are the first two members of a pattern: an agent's own environment
carries enough — a hook port and its durable task id — to call back into La Vigie and
act on itself, with no UI in the loop. The finish route's real work already lives in a
shared core (`prepare_teardown` / `perform_teardown` / `teardown_task`) precisely so it
isn't hard-wired to the self-teardown HTTP path — the split between a cancellation-safe
"prepare" phase and a run-to-completion "perform" phase exists so a second, differently
scoped caller (one tearing down a task other than its own, with no risk of killing its
own request) can drive the same safety gate and destructive sequence synchronously,
without the detach dance self-teardown needs. See [MCP server](./mcp-server.md) for the
other half of La Vigie's agent-facing control plane; a natural next self-service
endpoint could live there or alongside these two HookBridge routes. If you add one,
keep the pattern honest: key it on an id that survives a restart, document the
body/query contract next to the route, and make status codes mean what they say — a
`200` should mean "done or durably accepted," never "maybe."
