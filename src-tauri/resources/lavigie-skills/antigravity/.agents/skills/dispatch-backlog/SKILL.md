---
name: dispatch-backlog
description: >-
  Scan a project's low-priority todo backlog, pick a small set of genuine
  one-shot tickets, classify each for GUI-review need, dispatch them as La Vigie
  tasks, and wire the right completion chain per class. Conservative by design —
  errs toward NOT dispatching.
---
# Dispatch backlog: scan → score → classify → dispatch → wire completion

Package the recurring manual move — *"scan the low-priority backlog, find the easy self-contained
one-shots, and start a few"* — into one repeatable pass. It reads the backlog from **your session's
task tracker** and dispatches through **La Vigie** (`list_repos` / `list_tasks` / `start_task`).

**This is an orchestration skill:** run it from a session that can both (a) query its task tracker's
backlog and (b) call the `lavigie` MCP tools — your own session, a concierge, or a per-repo
orchestrator. It is **conservative by construction** — when a ticket is ambiguous, it is skipped,
not dispatched.

## Tracker seam (stay tracker-agnostic)

This skill never names a specific tracker. Two tracker capabilities are needed, and both come from
whatever task-tracker tools your session exposes (discover them with `ToolSearch`; do not assume a
backend):

- **List the backlog** — enumerate tickets filtered by project, priority, and status. *This is
  beyond the repo's `task-provider` 4-op contract (read / set-status / sync / comment), so it uses
  the tracker's own list capability directly.*
- **Read a ticket by key** — fetch one ticket's full description + acceptance criteria. (Also
  outside `task-provider`, whose `read` op only resolves the *current* worktree's task.)

The La Vigie side (`mcp__lavigie__list_repos` / `list_tasks` / `start_task`) is always concrete —
load those schemas with `ToolSearch` before calling.

## Inputs

- **Target** — the project/repo to scan. If the invocation names one, use it. Otherwise infer from
  context: this repo's tracker project and its La Vigie repo (`mcp__lavigie__list_repos`, match by
  name/path). If still ambiguous, **ask** — never guess a target you'll dispatch real work into.
- **Priority** — default `low` (the backlog tier this skill farms). Overridable.
- **Cap** — max tasks to dispatch this pass. Default **3**, hard ceiling. The caller may lower it;
  never silently raise it.

## Step 1 — Scan the backlog

Using your tracker's list capability, enumerate the candidate set filtered by:

- **project** = the target,
- **priority** = the chosen priority (default `low`),
- **status** = `todo` (this already excludes anything `in_progress`).

Keep the returned tickets as the raw candidate pool. If the pool is empty, report "nothing to
dispatch" and stop.

## Step 2 — Dedupe against already-started work

A ticket is **out** if it is already being worked, even if the tracker still says `todo` (status
can lag a launch). Pull the live La Vigie task list:

```
mcp__lavigie__list_tasks()
```

Drop any candidate whose ticket key (or derived branch — La Vigie seeds the branch from the ticket
key) matches an existing La Vigie task in any non-terminal state (pending / running / in_progress).
Also drop anything the tracker marked `in_progress`. Dedup is a hard filter — **when in doubt that
a ticket is already started, skip it** (a missed dispatch is cheap; a double-dispatch collides
worktrees).

## Step 3 — Score for one-shot-ability

For each surviving candidate, read its full description + ACs by key, then apply this **checklist**
(not a vibe — every box should be checkable from the ticket text):

**Favor (all should hold):**
- **Concrete prescribed fix** — the ticket names *what to change*, not just a problem. A named
  fix, the file(s) or symbol(s), the mechanism. *This is a hard gate (see Guardrails): a ticket
  that is only a problem statement is skipped, never one-shotted.*
- **Small blast radius** — a localized edit; ideally one or a few files; no wide refactor.
- **Clear acceptance criteria** — a concrete, checkable "done" bar.

**Deprioritize / skip:**
- Investigations ("figure out why…", "explore options for…") — no prescribed fix.
- Multi-option **design** tickets (the ticket itself lists open design questions to decide).
- Anything touching many files, migrations, auth, security, money/billing, data-loss, or
  session/terminal lifecycle — these are never one-shots (they're the full-path, human-in-the-loop
  classes a merge policy hard-excludes from auto-merge).

Rank the passing candidates best-first; keep the top **cap** for dispatch. Everything else becomes
a *skipped* row with its reason.

## Step 4 — Classify GUI-review need

Classify each ticket you'll dispatch (mirrors the repo guide's ship-time GUI exception):

- **No GUI review** — the change is backend / pure-function / unit-testable, fully verifiable via
  the repo's automated gate (tests + builds): parsers, helpers, MCP tool logic, DB migrations
  tested against fixtures, CI/workflow, thin glue.
- **Needs human GUI verify** — the change has visual/runtime surface that agents cannot verify
  headlessly: terminal rendering, live status indicators, modals, the folder picker, notifications,
  the PR flow, or any desktop-window behavior.
- **Unsure → treat as GUI.** Never auto-finish something that might need eyes.

## Step 5 — Dispatch (capped)

For each selected ticket, launch a La Vigie task with a **self-contained prompt** that names the
fix, the files, the verification bar, and the completion chain for its class:

```
mcp__lavigie__start_task(
  repo=<target repo id>,       # from list_repos; defaults to the caller's repo
  ticketKey="<ticket key>",    # links the task + seeds the branch name
  prompt=<self-contained prompt, see below>
)
```

The prompt must stand alone (the launched agent starts fresh). Include: a one-line statement of
the prescribed fix; the file(s)/symbol(s) named in the ticket; the verification bar (which
tests/builds must pass); and the **completion-chain instruction for its class** —

- **No GUI review →** end the prompt with: *"When the change is green (tests + build), run
  `/ship` → `/await-merge` → `/finished` — fully autonomous; tear yourself down on merge."*
- **Needs GUI review →** end the prompt with: *"When the change is green, run `/ship` only, then
  STOP: leave the task `in_progress` with `PENDING: human GUI verification`. Do NOT run
  `/finished` — a human must verify the GUI first."* (Matches the repo guide's ship exception.)

Stop at the cap. If more than `cap` tickets passed scoring, dispatch the top `cap` and list the
rest as *deferred (over cap)* in the report — do not silently drop them.

## Step 6 — Report

Emit a concise report:

- **Dispatched** — one row per launch: ticket key, title, class (no-GUI / GUI), and the completion
  chain it was given.
- **Skipped** — one row per non-dispatch with the reason (already started, no concrete fix,
  investigation, multi-option design, too-broad/excluded class, over cap).
- **Caveats** — always restate the two you cannot design away:
  - **Overlapping files** — concurrent tasks that touch the same files will hit rebase conflicts;
    if two selected tickets obviously overlap, dispatch one and defer the other.
  - **Merge-order nondeterminism** — under GitHub's up-to-date-branch requirement, auto-merge
    order across the dispatched batch is nondeterministic; later PRs may need a rebase before they
    can land.

## Guardrails (do not violate)

1. **Cap is a ceiling** — never dispatch more than `cap` (default 3); the caller may lower it.
2. **Require a concrete prescribed fix** — a problem-statement-only ticket is skipped, not
   one-shotted.
3. **Dedupe** — never dispatch a ticket already `in_progress` or already a live La Vigie task.
4. **Never auto-`/finished` a GUI task** — GUI-class dispatches get `/ship` + hold only.
5. **Err toward NOT dispatching** — ambiguity resolves to *skip*, and the report says why.
