# Task dependencies (start-on-merge)

Queue a task to **auto-start when another task lands**. Instead of babysitting a follow-up — waiting
for the first task's PR to merge, then manually creating and starting the next one — you dispatch the
follow-up up front as *queued*, and La Vigie starts it the moment its dependency lands.

## Queuing a task

A running agent queues a follow-up through the La Vigie MCP tool `start_task`, passing
`afterMergeOf` — the task(s) it should wait for (typically including the agent's own current task,
from `LAVIGIE_TASK_ID`). `afterMergeOf` accepts **a single task id or an array of ids**:

```
start_task(title: "Follow-up work", afterMergeOf: "<blocking task id>", prompt: "…")
start_task(title: "Integration",    afterMergeOf: ["<task A id>", "<task B id>"], prompt: "…")
```

A task queued behind several blockers stays queued until **all** of them land, then auto-starts
**exactly once** (see *Multiple blockers* below). Passing a single id and passing a one-element
array behave identically. Any listed id that no longer exists (already merged-and-gone) is dropped;
if *every* listed id is already gone, the task launches immediately instead of queuing.

The new task is created in a **Queued** state:

- a task row exists and shows in the sidebar with a distinct *queued* dot (plus a `⛓ N` badge
  counting its outstanding blockers), and
- its detail pane lists the **actual tasks it's waiting on** — not a blanket message — shrinking as
  each blocker lands, but
- it has **no worktree, no branch, and no agent** yet.

`start_task` confirms the queue rather than pretending it started, naming every blocker:

> Queued task `<id>` (Pending) — will auto-start once all of these land: `<blocker>` (`<title>`),
> `<blocker2>` (`<title2>`). No worktree/branch yet; it unblocks automatically when each blocker's PR
> merges (detected at finish/teardown), or immediately via `/finished` with `promote=true` for a
> no-PR landing.

## What "landed" means, and when dependents fire

Queued tasks start when their dependency has **landed** — its work is in the base branch. La Vigie
checks this when you **finish** the dependency (through *any* finish surface: the GUI **Finish**
button, or the `/finished` teardown):

- **The dependency has a PR** → landed is detected automatically when that PR is **merged** — however
  it was merged (the GitHub merge button, auto-merge, a merge script, or La Vigie's own merge). You
  don't have to merge *through* La Vigie; you just have to finish the task afterwards.
- **No PR** → nothing is auto-detected, so a queued dependent stays queued. If the work landed
  without a PR and you want the dependents to start, assert it explicitly with the **promote bypass**
  (see below).

```mermaid
flowchart TD
  F["You finish a task<br/>(GUI Finish, or /finished)"] --> D{"Any tasks queued<br/>on it?"}
  D -- No --> Z["Nothing extra happens"]
  D -- Yes --> L{"Did its work land?<br/>(PR merged, or promote bypass)"}
  L -- "No — not merged, no bypass" --> Q["Queued tasks keep waiting"]
  L -- Yes --> P["Each queued task auto-starts:<br/>fresh worktree off the updated base<br/>+ its agent begins with the seeded prompt"]
```

The typical flow — merge the PR wherever you like, then finish the task:

```mermaid
sequenceDiagram
  actor You
  participant GH as GitHub
  participant LV as La Vigie
  You->>LV: start_task(afterMergeOf: A) → task B is Queued
  Note over LV: B waits — no worktree/agent yet
  You->>GH: merge A's PR (button, auto-merge, or a script)
  You->>LV: finish A (GUI Finish, or /finished)
  Note over LV: La Vigie sees A's PR is merged (landed)
  LV-->>You: B auto-starts — worktree off the updated base + agent running
```

A promoted task's worktree is created off the **freshly-updated** base branch, so it already contains
the dependency's merged work.

## Multiple blockers

A queued task can wait on **more than one** blocker (`afterMergeOf: ["A", "B"]`). It stays queued
until the **last** of them lands, then promotes **exactly once**:

- Landing a *strict subset* of its blockers does **not** start it — when you finish blocker A, La
  Vigie clears A's edge but the task keeps waiting on B, and only promotes once B lands too.
- Its sidebar `⛓ N` badge and detail-pane blocker list update as each blocker lands, so you can see
  exactly what it's still waiting on.
- Promotion stays idempotent (it fires once, even across multiple finish surfaces) and a failure to
  start one queued task never affects the others or the task you just finished.

```mermaid
flowchart TD
  Q["Task queued on A and B"] --> FA["You finish A (landed)"]
  FA --> C1{"All blockers landed?"}
  C1 -- "No — still waiting on B" --> W["Stays queued (badge → ⛓ 1)"]
  W --> FB["You finish B (landed)"]
  FB --> C2{"All blockers landed?"}
  C2 -- "Yes" --> P["Auto-starts once: worktree off the updated base + agent begins"]
```

## The promote bypass (no-PR landings)

When work lands without a La Vigie-visible PR merge, pass `promote=true` so queued dependents start:

- via the `/finished` skill's promote option, or
- directly against the HookBridge:

  ```bash
  curl -s -X POST "http://127.0.0.1:$LAVIGIE_HOOK_PORT/finish/<taskId>?force=true&promote=true"
  ```

  (`force=true` skips the teardown safety gate for a task with unmerged commits and no PR;
  `promote=true` asserts the work landed and starts its dependents.)

## Cancelling a queued task

A queued task has no worktree, so cancelling it is just a delete — right-click it in the sidebar →
**Delete**. It disappears with no git teardown.

## Notes & limitations

- **One or more dependencies per task** (`afterMergeOf` takes a single id or an array). A task with
  several blockers promotes only after *all* of them land (see *Multiple blockers* above).
- If a dependency is finished **without landing** (e.g. discarded, or kept without a merged PR and no
  promote bypass), its queued dependents stay queued rather than starting — cancel them manually if
  they're no longer wanted.
- Fully hands-off firing — promoting dependents on a PR merge you *never* finish in La Vigie — is a
  planned follow-up (external-merge polling); today the trigger runs when you finish the dependency.
