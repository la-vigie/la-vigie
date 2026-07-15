---
name: finished
description: >-
  Wrap up a La Vigie worktree session — tear it down via La Vigie's HookBridge
  (stops the agent, removes the worktree, deletes the task).
allowed-tools: Bash
---
The user is done with this worktree session. Call the HookBridge self-teardown endpoint, which stops this agent's PTY, removes its worktree, and deletes its task row. It is keyed on the durable **task_id** (TASK-151), so it resolves even after La Vigie restarted since this agent launched. Prefer `LAVIGIE_TASK_ID`, falling back to `LAVIGIE_AGENT_ID` for an older La Vigie build that predates it. This is your **last action** — a `200` kills this process mid-request, so do not output anything after it.

**Start-on-merge dependents (TASK-90):** if other tasks were queued on this one (dispatched with `afterMergeOf: <this task>`), La Vigie promotes them when this task's work has **landed**. A merged PR is auto-detected at teardown — so for the normal flow you do **nothing**. Add **`promote=true`** to the query only when the work landed **without a La Vigie-visible PR merge** (there's no PR, or the PR was merged entirely outside La Vigie) and you want those dependents to start now. Set `PROMOTE` below accordingly — default empty; use `promote=true` only when you (or the user) are asserting the work has landed. It is ignored by older La Vigie builds, so it is always safe to include when warranted.

```bash
LAVIGIE_ID="${LAVIGIE_TASK_ID:-$LAVIGIE_AGENT_ID}"
PROMOTE=""                     # or PROMOTE="promote=true"  — see note above
Q=""; [ -n "$PROMOTE" ] && Q="?$PROMOTE"
code=$(curl -s -o /tmp/lavigie_finish_body -w "%{http_code}" -X POST \
  "http://127.0.0.1:$LAVIGIE_HOOK_PORT/finish/$LAVIGIE_ID$Q")
echo "status=$code"; cat /tmp/lavigie_finish_body 2>/dev/null
```

Interpret the result:
- **200** — done. The task was torn down (and any landed dependents were promoted); stop here.
- **409** — refused: the worktree has uncommitted changes or unmerged commits (the body says which). The PTY is still alive. Report the reason to the user. Only if they explicitly confirm discarding the work, re-run the same `curl` with `force=true` in the query (e.g. `?force=true`, or `?force=true&promote=true` to also fire dependents).
- **404** — the id is unknown to La Vigie (already torn down, a rootless/concierge session that never had a task, or a stale env). Report it.
