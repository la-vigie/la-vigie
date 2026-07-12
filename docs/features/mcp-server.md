# MCP server

## What it is

La Vigie runs an in-process [MCP](https://modelcontextprotocol.io) server so an agent
working inside one task can drive the rest of the cockpit programmatically — dispatch a
new task, list repos, and (for a broader-scope caller) check on other tasks in flight.
It's the twin of the desktop UI: everything it does goes through the same backend paths
as the New Task form and the sidebar.

The server speaks JSON-RPC over a loopback HTTP endpoint on an ephemeral port, authenticated
with a bearer token. Every running agent gets an MCP config pointing at it automatically —
there's nothing to install or configure by hand.

## How to use it

Five tools are registered:

- **`start_task`** — create and start a new task (worktree + branch + agent). All
  arguments are optional: `title` (human title), `ticketKey` (provider ticket id — also
  seeds the branch name), `repo` (target repo id; defaults to the calling agent's own
  repo), `agent` (which agent engine to launch; defaults to the repo/global default), and
  `prompt` (an initial prompt for the new agent, combined with the repo's configured
  prompt).
- **`list_repos`** — list the repos registered in La Vigie, so you can pick or confirm a
  `repo` id for `start_task`. No arguments.
- **`list_tasks`** — list every task across all repos with its current status. No
  arguments.
- **`task_status`** — get one task's status and metadata by id. Argument: `taskId`
  (required).
- **`get_task_activity`** — read a task's recent agent conversation as chat-shaped
  messages. Arguments: `taskId` (required) and `since` (optional byte-offset cursor from
  a prior call — omit or pass `0` to read from the start). The result includes a `cursor`
  you pass back as `since` to poll incrementally.

## How it works

Each bearer token carries one of two tiers, and the tiers are scoped in opposite
directions — one can *act*, the other can *read broadly*:

- **Agent tier.** Every agent La Vigie launches gets its own token, bound to the task it
  started in. It can call `start_task` (defaulting to its own repo unless it passes a
  `repo` override) and `list_repos`, but `list_tasks`, `task_status`, and
  `get_task_activity` all reject it — those calls require a concierge-scope token.
- **Concierge tier.** A single broad-scope token used by the (separate) concierge
  session, which is not tied to any one task. It can call `list_tasks`, `task_status`,
  `get_task_activity`, and `list_repos` across every repo and task, but `start_task`
  refuses it outright — dispatching new work is intentionally left to task-scoped agents,
  not the cross-task reader.

An unrecognized or missing bearer token gets a `401`. Tool calls that are the wrong shape
(e.g. a missing `taskId`) come back as a JSON-RPC error; calls that are well-formed but
fail at runtime (e.g. an unknown task id) come back as a successful RPC response whose
tool result is flagged `isError`, so the calling model sees a normal tool failure rather
than a transport error.

## Extending it

The tool set is the seam: adding a new capability to the control plane means adding a new
`tools/list` entry and a matching branch in the dispatcher, following the same
pure-routing / async-glue split as the rest of the server — request parsing and response
shaping are plain functions, and the only async code is the handful of side effects
(launching a task, reading the store, reading a session transcript).
