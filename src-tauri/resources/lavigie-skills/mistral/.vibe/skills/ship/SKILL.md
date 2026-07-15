---
name: ship
description: >-
  Create a PR, sync the task, and mark it done — the full ship flow with a
  blocking fresh-context review gate. Tracker writes go through the repo's
  task-provider (skipped if none).
---
# Ship: review gate + PR + task-sync + done

Commit, pass a fresh-context review gate, push, create a PR, then sync + close the task through
this repo's `task-provider` — all in one go. The gate is BLOCKING: no PR while Important findings
are open. This skill never names a tracker; the sync + done steps go through `task-provider` and
are skipped when the repo commits no adapter.

**Fast-path tasks** (a one-sentence diff shipped without a spec — see `/lavigie:worktree-init`)
run this exact flow; the spec-dependent substeps degrade gracefully — there's no decision log to
sync. Ship on the strength of the gate + CI, and record nothing by hand that git already records.

## Steps

1. **Determine the task ID** from the branch: `git branch --show-current` (e.g. `TASK-7` from
   `task-7-fix-bug`). Used for the PR title scope and to locate the spec file.

2. **Commit uncommitted changes** (`git status`): stage relevant files (never `.env`/credentials),
   one commit in the repo's style (`git log --oneline -5`), ending with the repo's standard
   `Co-Authored-By: Claude <model> <noreply@anthropic.com>` trailer.

3. **Review gate (blocking, fresh context)** — after the commit, before any push.
   - Base branch: `gh repo view --json defaultBranchRef -q .defaultBranchRef.name` (fallback `main`).
   - Look for `REVIEW.md` at the repo root then `.claude/REVIEW.md`; its contents are the
     highest-priority review instructions.
   - Dispatch ONE fresh reviewer via the Agent tool (`subagent_type: general-purpose`) with ONLY:
     the repo path, base branch, and an instruction to generate/read `git diff <base>...HEAD`
     itself; the spec/decision-log path if one exists; the repo's CLAUDE.md/AGENTS.md path; the
     full text of REVIEW.md; and these rules: review the diff against spec + repo invariants;
     classify each finding **Important** (bug, spec/invariant violation, security, data loss —
     blocks merge) or **Nit** (report at most FIVE); every behavior claim needs a `file:line`
     citation; do NOT propose alternative implementations; end with exactly one line:
     `review-gate: {"important": <n>, "nits": <n>}`.
   - Do NOT share your session's reasoning or opinion — spec + diff + rules only.
   - **If `important > 0`: STOP.** No push, no PR. Present findings, fix (or dispatch a fix), re-run
     the gate fresh. Only if the user explicitly says to ship anyway may you proceed — then record
     `Review gate: OVERRIDDEN by user (<n> Important open)` in the PR body.
   - On pass, keep the verdict for step 6 and append a one-line gate result to the spec log if one exists.

4. **Documentation scan (advisory, non-blocking)** — after the gate, before pushing. If the repo
   defines a `scan-for-doc-change` skill (glob `.claude/skills/scan-for-doc-change/SKILL.md` or
   `.claude/commands/scan-for-doc-change.md`), invoke it (or Read + follow it inline if it isn't
   registered yet). Read its `doc-scan:` line; if `userFacing` is true and `documented` is false,
   surface the advisory + drafted entry and OFFER to write it into the named doc on this branch
   (fold into the branch before pushing). Never blocks. If no such skill, skip silently.

5. **Push** the branch to origin with `-u` if not already tracking.

6. **Create a PR** with `gh pr create`:
   - Title: `type(TASK-ID): short description` — conventional-commits, task ID in scope, <70 chars.
     Types: `feat`, `fix`, `refactor`, `chore`, `docs`, `test`.
   - Body (HEREDOC):
     ```
     ## Summary
     <1-3 bullets>

     ## Test plan
     <checklist>

     ## Review gate
     PASS — 0 Important, <n> nits (fresh-context reviewer, diff vs <base>)

     🤖 Generated with [Claude Code](https://claude.com/claude-code)
     ```
   - **Post the gate verdict as a commit status** so a merge policy / native auto-merge can require
     it (see the repo's `MERGE_POLICY.md` if present). On the head SHA:
     ```bash
     gh api "repos/{owner}/{repo}/statuses/$(git rev-parse HEAD)" \
       -f state=success -f context=review-gate \
       -f description="0 Important, <n> nits — fresh-context reviewer"
     ```
     Use `state=success` only for a genuine PASS. If the user overrode Important findings, post
     `state=failure` with the count. Skip only if `gh` lacks `statuses` write scope.

7. **Sync the task** — invoke `task-provider` **sync(text)** with the spec decisions from
   `memory/spec_<TASK_ID>.md`. **Skip on a fast-path task** (no spec file) and if `task-provider`
   returns **none** — both are expected, not errors.

8. **Mark the task done** — invoke `task-provider` **set-status(done)**. Skip if `task-provider`
   returns **none**.

9. **Report** the PR URL, the gate verdict, and whether the task was synced + marked done (or
   that there was no task-provider, so those were skipped).
