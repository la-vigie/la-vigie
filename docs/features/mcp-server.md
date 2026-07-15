# MCP server

## What it is

La Vigie runs an in-process [MCP](https://modelcontextprotocol.io) server so an agent
working inside one task, or a per-repo orchestrator session, can drive the rest of the
cockpit programmatically — dispatch or finish a task, schedule one for later, list repos,
and (for a broader-scope caller) check on other tasks in flight. It's the twin of the
desktop UI: everything it does goes through the same backend paths as the New Task form
and the sidebar.

The server speaks JSON-RPC over a loopback HTTP endpoint on an ephemeral port, authenticated
with a bearer token. Every running agent gets an MCP config pointing at it automatically —
there's nothing to install or configure by hand.

## How to use it

Fourteen tools are registered, grouped by what they let a caller do:

**Act — task lifecycle**
- **`start_task`** — create and start a new task (worktree + branch + agent). All
  arguments are optional: `title` (human title), `ticketKey` (provider ticket id — also
  seeds the branch name), `repo` (target repo id; defaults to the calling token's own
  repo — an explicit cross-repo `repo` is denied), `agent` (which agent engine to launch;
  defaults to the repo/global default), `model` (an optional model override for engines
  that take one, e.g. `--model opus`; omitted ⇒ the engine's default), `prompt` (an
  initial prompt for the new agent, combined with the repo's configured prompt), and
  `afterMergeOf` (queue the task to start once another La Vigie task merges).
- **`queue_dependency`** — a dependency-first alternative to `start_task`'s `afterMergeOf`:
  create a NEW task **queued** behind other tasks, which auto-starts only once all of them
  merge. The `dependsOn` argument (one La Vigie task id or an array) is **required** — so,
  unlike `afterMergeOf`, you can't accidentally launch now by forgetting it. The other
  arguments (`title`, `ticketKey`, `repo`, `agent`, `model`, `prompt`) and the repo
  scoping match `start_task`. The task stays `Pending` (no worktree/agent yet) until released.
- **`finish_task`** — tear down a task: stop its agent, remove the worktree, delete the
  task. Defaults to the caller's own task; targeting another (`taskId`) requires an
  orchestrator-scope token for that task's repo. `mode` is `keep` (default), `discard`,
  or `merge` (squash-merge then delete the branch); refuses uncommitted/unmerged work
  unless `force` is set.
- **`send_task_message`** — "stir" another task: deliver a `message` to a task's
  running agent and submit it, to unblock, redirect, or nudge a waiting agent.
  Arguments: `taskId` (required) and `message` (required). Repo-scoped — you may
  only message a task in your own repo; the concierge is denied. Requires a live
  agent: if the task has no running agent it errors (start or resume it first).
  Read the agent's reply afterward via `get_task_activity`.

**Act — scheduling**
- **`schedule_task`** — a one-shot deferred launch: fire once at a future time (`inHours`
  / `inSeconds` / `atUnix`), then retire. Defaults to the caller's own repo. Also takes
  `title`, `prompt`, `agent`, and an optional `model` override, mirroring `start_task`.
- **`create_schedule` / `list_schedules` / `update_schedule` / `set_schedule_enabled` /
  `delete_schedule`** — manage recurring cron-driven schedules for a repo (added by
  TASK-173/TASK-178). All confined to the caller's own repo; a schedule's repo is resolved
  from its own row for the id-addressed calls.

**Read**
- **`list_repos`** — list the repos registered in La Vigie, so you can pick or confirm a
  `repo` id for `start_task`. No arguments; the only ungated tool.
- **`list_tasks`** — list tasks with their current status. A repo-scoped caller
  (Agent/Orchestrator) sees only its own repo's tasks; the concierge sees every repo.
- **`task_status`** — get one task's status and metadata by id. Argument: `taskId`
  (required).
- **`get_task_activity`** — read a task's recent agent conversation as chat-shaped
  messages. Arguments: `taskId` (required) and `since` (optional byte-offset cursor from
  a prior call — omit or pass `0` to read from the start). The result includes a `cursor`
  you pass back as `since` to poll incrementally.

## How it works

Every bearer token carries one of three tiers. Every mutating or read-gated tool routes
through a single choke-point, `mcp::authz::decide()` — **`src-tauri/src/mcp/authz.rs` is
the source of truth**: `registry()` maps each such tool to a `Capability` and a
`ResolutionStrategy` that says how the *target* repo is resolved (from an arg, from the
task/schedule row, or the caller's own repo) — always from storage, never trusted from a
caller-supplied id, so a caller can't lie its way onto another repo's resource. An
exhaustiveness test fails CI if a new acting tool is added without a registry entry
(deny-by-default).

- **Agent tier.** Every agent La Vigie launches gets its own token, bound to the task and
  repo it started in. It can call `start_task`/`finish_task`/`schedule_task` (defaulting
  to its own repo/task; an explicit cross-repo target is denied) and the read tools, but
  only for its own repo.
- **Orchestrator tier** (`Orchestrator{repo_id}`, added by TASK-180). The token behind a
  per-repo, worktree-less orchestrator session — the desktop's pinned "Orchestrator" row
  (TASK-126) and the remote `POST /api/orchestrator/{repoId}` route both mint/use one. It
  is the acting tier for its repo: the same task-lifecycle, scheduling, and read
  capabilities as the Agent tier, confined to its own `repo_id` by the same deny-by-default
  registry. Cross-repo acting is denied — there is no escape hatch.
- **Concierge tier.** The legacy global session (rootless, not tied to any task or repo)
  is now **read-only**: `list_tasks`, `task_status`, `get_task_activity`, and `list_repos`
  across every repo, but every acting capability — `start_task`, `finish_task`,
  `schedule_task`, and all schedule management — is denied outright. TASK-178 originally
  let the concierge schedule any repo; **TASK-180 revoked that grant** — scheduling (and
  all other acting) now requires a repo-scoped Orchestrator or Agent token.

An unrecognized or missing bearer token gets a `401`. Tool calls that are the wrong shape
(e.g. a missing `taskId`) come back as a JSON-RPC error; calls that are well-formed but
fail at runtime (e.g. an unknown task id, or an authz denial) come back as a successful
RPC response whose tool result is flagged `isError`, so the calling model sees a normal
tool failure rather than a transport error.

## Extending it

The tool set is the seam: adding a new capability to the control plane means adding a new
`tools/list` entry, a matching branch in the dispatcher, and — for any acting or
read-gated tool — an entry in `mcp::authz::registry()` (the exhaustiveness test enforces
this), following the same pure-routing / async-glue split as the rest of the server —
request parsing, response shaping, and the authz decision are plain functions, and the
only async code is the handful of side effects (launching a task, reading the store,
reading a session transcript).
