---
name: await-merge
description: >-
  Arm auto-merge on a PR, then poll in the background until it merges or a gate
  fails — waking me only on a terminal event
allowed-tools: 'Bash, Skill'
argument-hint: '[PR number or URL]'
---
# Await merge: arm auto-merge, then watch until done

Arm native GitHub auto-merge on a PR and start a **background** poller that only
returns — and re-invokes me — when the PR reaches a terminal state: **merged**, a
**gate/check failed**, a **merge conflict with the base**, **closed**,
**green-but-won't-self-merge**, or a safety **timeout**. While checks are still
running (or green with auto-merge armed and
GitHub about to merge), it just sleeps and polls again — it does NOT wake me.

The mechanism: the poll script runs under `run_in_background`, and this harness
re-invokes me when a background command *exits*. So the script's exit conditions
are exactly my wake conditions. Pairs with `/ship` (which posts the `review-gate`
status the poller waits on) and `/check-pr` (a one-shot readiness check).

## Steps

1. **Resolve the PR.** If `$ARGUMENTS` is a PR number/URL, use it; otherwise the
   script auto-detects the PR for the current branch. Grab the repo dir so the
   detached background process has git/gh context:
   ```bash
   git rev-parse --show-toplevel   # -> REPO_DIR
   gh pr view ${ARGUMENTS:+"$ARGUMENTS"} --json number,url,state -q '.number, .state, .url'
   ```
   If the PR is already `MERGED`/`CLOSED`, report it and stop — nothing to watch.

2. **Launch the poller in the background.** Call the Bash tool with
   `run_in_background: true` so it detaches and re-invokes me on exit. Pass the
   repo dir explicitly (a detached process must not rely on cwd), and the PR
   number if known:
   ```bash
   bash "${CLAUDE_PLUGIN_ROOT}/skills/await-merge/poll-pr-until-merged.sh" <PR> --dir "<REPO_DIR>"
   ```
   Optional flags: `--method merge|squash|rebase` (default: auto-detected from
   the repo's allowed methods, preferring merge → squash → rebase, so arming
   auto-merge doesn't fail on repos that disable merge commits), `--interval`
   secs (30), `--timeout` secs (2700 = 45 min), `--no-automerge` (skip arming
   auto-merge and only watch). Tell the user it's watching in the background and
   that they can keep working — I'll report back when it resolves.

3. **When the poller exits, I'm re-invoked.** Read its final `poll-result: {…}`
   line (and exit code) and act on the `outcome`:
   - **`merged`** (exit 0) — report it merged. Then invoke the `task-provider` skill's
     **set-status(done)** op to close the task in this repo's tracker (best-effort: if
     `task-provider` returns **none**, i.e. the repo commits no adapter, silently skip — the
     merge already succeeded). Finally, suggest `/lavigie:finished` to tear the worktree down.
   - **`failed`** (exit 1) — a required gate failed. Surface the failed check
     name(s) from the result line, pull the failing logs (`gh pr checks <PR>`,
     `gh run view --log-failed`), and offer to investigate/fix. Do NOT silently
     re-launch.
   - **`conflict`** (exit 6) — the PR has merge conflicts with the base branch
     (`mergeStateStatus: DIRTY` / `mergeable: CONFLICTING`). This is CI-invisible
     — checks can be all-green while the branch is unmergeable — so the poller
     surfaces it instead of spinning until timeout. Report it and offer to
     update/rebase the branch onto the base, resolve the conflicts, then
     re-launch the poller. Do NOT silently re-launch.
   - **`green-no-automerge`** (exit 4) — all checks are green but the PR won't
     merge itself (`reason`: auto-merge not armed, review required, or changes
     requested). Report the reason. If it's just that auto-merge isn't available
     (no branch protection), offer to merge directly with a repo-allowed method
     (`gh pr merge <PR> --squash|--merge|--rebase`). If review is required, say
     so — a human needs to approve.
   - **`closed`** (exit 2) — report the PR was closed without merging.
   - **`timeout`** (exit 3) — still pending after the window. Summarize current
     status (`/check-pr`) and offer to re-launch the poller (optionally longer
     `--timeout`).

## Notes

- Advisory/observational: the poller never force-merges or overrides a failing
  gate — it either lets GitHub's auto-merge do the merge or wakes me to decide.
- One-shot readiness (no waiting) → use `/check-pr` instead.
- The script lives at `${CLAUDE_PLUGIN_ROOT}/skills/await-merge/poll-pr-until-merged.sh`;
  run it with `--help` for the full flag/exit-code reference.
