# Tasks and worktrees

## What it is

The durable unit in La Vigie is a **task**: a git worktree plus its branch. The left
sidebar lists every repository you've added and, under each one, its tasks — so you can
run several agents on the same repo (or several repos at once) without juggling terminal
tabs or manually managing worktrees yourself.

## How to use it

**Add a repository** with the **+** button in the sidebar header; it opens a folder
picker and registers the chosen directory.

**Create a task** with the **New task** button next to a repository. The form needs a
**task name**, which gets slugified into the branch name (e.g. "Add People CRUD tools" →
`add-people-crud-tools`). You can also give it an optional **ticket ID** — if set, the
branch is slugified from the ticket ID instead of the title — and an optional **base
branch** (defaults to the repo's default branch). You can type a prompt to send the agent
the moment it starts, pick a saved prompt from the library, choose the agent engine and
model, and leave **Start the agent immediately** checked to launch everything in one go.

Once created, the task shows up in the sidebar with a live status dot, its ticket key (if
any), and badges for a running/failed background setup or an open PR.

**Hide / show a task**: right-click a task and choose **Hide** to tuck it out of the main
list into a collapsible "N hidden" section at the bottom of the repo — useful for tasks
you're not actively working but don't want to delete yet. Right-click a hidden task and
choose **Reopen** to bring it back.

**Finish a task**: open it and click **Finish task** in the header. You'll get up to
three options — **Merge PR & finish** (only shown when the task has an open PR), **Keep
branch**, and **Discard branch** — plus **Cancel**.

**Delete a task**: right-click it and choose **Delete**. A confirmation dialog lets you
optionally also delete the branch; this is unconditional (no PR/merge step) and can't be
undone.

## How it works

Task **creation** runs the same backend path whether you use the New task form or an
agent calls `start_task` (see [MCP server](./mcp-server.md)), so both behave identically.
Finishing and deleting are driven from the UI; an agent tearing down *its own* task uses
a shared teardown core (see [Agent self-management](./agent-self-management.md)).

- **Creating** a task first resolves the branch name (slugified ticket key, or slugified
  title if there's no ticket key) and the base branch, then runs `git worktree add` off
  that base. The task row is inserted into the local SQLite database *before* any repo
  setup step runs, so the task is immediately visible and navigable. Setup (a per-repo
  setup command, or a committed `.vigie/setup.sh` script) then runs as a non-blocking
  background job, streaming its output into a small dismissible status strip in the task
  header rather than blocking the UI.
- **Hiding** a task only flips a `hidden` flag on its database row — the worktree and
  branch are untouched. It's purely a sidebar decluttering tool, not a teardown.
- **Finishing** a task stops any running agent sessions in it, optionally merges its PR
  (for "Merge PR & finish"), removes the worktree with `git worktree remove --force`,
  deletes the task's database row, and — for "Discard branch" or after a merge — makes a
  best-effort attempt to delete the branch too (failures there are ignored, since the
  worktree and task record are already gone). "Keep branch" skips branch deletion so you
  can still find and check it out later.
- **Deleting** a task does the same worktree-removal and row-deletion as finishing, but
  without a merge step and without needing a PR — it also cancels any background setup
  job still running against that worktree before removing it, and branch deletion is an
  explicit opt-in checkbox in the confirmation dialog rather than tied to a mode.

## Extending it

Tasks can also be created programmatically by an agent, without going through the
sidebar — see the MCP server's `start_task` tool in [MCP server](./mcp-server.md). An
agent can also tear down *its own* task from inside its session, without any UI, via the
HookBridge finish endpoint — see [Agent self-management](./agent-self-management.md).
