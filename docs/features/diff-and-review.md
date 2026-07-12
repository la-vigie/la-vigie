# Diff and review

## What it is

The **Changes** panel in the task workspace's right pane lets you review a task's
worktree, GitHub-style, without leaving the app: a file checklist, a syntax-highlighted
diff, staging and committing, and inline comments you can hand straight to the agent as
a follow-up prompt. A **Spec / Docs** dock underneath renders the task's Markdown specs
and design docs inline, so the plan the agent is working from stays one click away.

## How to use it

Open a task and look at the **Changes** header — a two-way toggle switches the diff
between:

- **Uncommitted** — the working tree vs the last commit (`HEAD`): what you could stage
  and commit right now. Each changed file has a checkbox; check the ones you want, type
  a commit message in the box at the bottom, and click **Commit**.
- **Compared to `<base>`** — the whole branch's diff vs its base branch, read-only (no
  checkboxes, no commit box). This is what a reviewer sees before merging.

Both sides show a file-count badge and a checklist of changed files above the diff
itself; expand a file to read the unified diff with GitHub-style syntax highlighting.

Comment on the **Compared to `<base>`** diff the way you'd review a pull request: click
a line to open a comment composer anchored to that exact line, write your feedback, and
save it. Saved comments accumulate in a footer bar at the bottom of the panel ("N
comments pending"). From there you can **Discard** them or click **Submit to Claude →**
to send them all to the agent in one prompt — the panel clears the queue once the prompt
is delivered.

Below the diff, the **Spec / Docs** dock (collapsible, drag-resizable, and expandable to
fill the whole pane) renders the task's own spec plus any design/plan Markdown that's new
on the branch. If there's more than one document it shows a picker; each entry is
labeled by kind (e.g. "Spec (TICKET-ID)", "Design: ...", "Plan: ...").

## How it works

**Diff computation.** Both the file list and the diff text are computed by shelling out
to `git` in the task's worktree — no diffing logic runs in the frontend. `Uncommitted`
always diffs against `HEAD` (plus untracked files, surfaced separately since `git diff`
doesn't see them). `Compared to <base>` diffs against the task's base branch.

**Fresh base fetch.** For the `Compared to <base>` scope, if the *"fetch remote base
before diffing"* setting is effectively on (a per-repo override, falling back to a
global app setting, falling back to a default) and the repo has a remote, the app
resolves the comparison ref to `origin/<base>` instead of the local base branch. It does
this without ever blocking the diff on the network: resolving the ref uses whatever
`origin/<base>` tracking ref is already known locally, while a throttled background
fetch (`git fetch origin <base>:refs/remotes/origin/<base>`) refreshes that ref for the
*next* render. If there's no tracking ref yet, no remote, or the setting is off, it
falls back to the local base branch — the diff always resolves, online or offline.

**Comments → prompt.** A saved comment carries the file path, side (old/new), line
number, the line's text, and your comment body. Submitting composes them into a single
prompt — "Please address these review comments:" followed by one `file:line — body` line
per comment — and delivers it to the task's agent terminal as a pasted (not
auto-submitted) prompt, starting the agent session first if none is running yet.

**Spec / Docs resolution.** The dock lists the task's own spec (a per-worktree file keyed
by the task's ticket ID, if the task has one) plus any Markdown under the repo's
design/plan directories that is new on the branch — i.e. not yet present on the base
branch. Reading a doc is checked against that same resolved list, so only files the dock
would show are ever readable.
