---
name: worktree-init
description: >-
  Initialize a worktree session — read the task via the repo's task-provider (or
  the launch prompt if none), rename this La Vigie task, set it in progress,
  then take the fast path (one-sentence diffs) or a full spec.
---
# Worktree Init: bootstrap a task session

Initialize the current worktree: read the task through this repo's `task-provider`, choose the
fast path (one-sentence diffs) or a full spec, and optionally start work. This skill never names
a tracker — all task reads/writes go through `task-provider`, which degrades to **none** when the
repo commits no adapter.

## Steps

1. **Read the task.** Invoke the `task-provider` skill's **read** op.
   - If it returns a task → keep its `title`, `description`, `acceptance_criteria`, and `status`.
   - If it returns **none** (no adapter in this repo) → read the task from the **launch prompt**
     you were given at startup; skip step 3's tracker write; still do the rename in step 2
     using the title from the launch prompt, and continue.

2. **Rename this La Vigie task** to the task's title (TASK-40). La Vigie injects
   `LAVIGIE_HOOK_PORT` + `LAVIGIE_TASK_ID` (durable key, TASK-151) + `LAVIGIE_AGENT_ID`. POST the
   new name (plain-text body) to the HookBridge rename endpoint keyed on the task_id. Use just
   the title — no ticket-ID prefix (La Vigie shows the ticket as a separate chip, TASK-16). Skip
   silently if the env vars are absent:
   ```bash
   LAVIGIE_ID="${LAVIGIE_TASK_ID:-$LAVIGIE_AGENT_ID}"
   if [ -n "$LAVIGIE_HOOK_PORT" ] && [ -n "$LAVIGIE_ID" ]; then
     curl -s -X POST \
       "http://127.0.0.1:$LAVIGIE_HOOK_PORT/rename/$LAVIGIE_ID" \
       --data-binary "<task title>"
   fi
   ```
   **200** echoes the applied name (renamed); **404** unknown id; **400** blank name.

3. **Set the task in progress.** Only if the `status` from step 1's read is `todo`, invoke
   `task-provider` **set-status(in_progress)**. If the status is anything else (already
   `in_progress`, `done`, `cancelled`, …) do NOT change it — never flip a finished task back.
   If task-provider returned **none** in step 1, skip this entirely.

4. **Choose the path — fast or full.** *If you can honestly describe the entire change in one
   sentence, take the fast path.* This is sanctioned, not a shortcut to feel guilty about.

   **Fast path** — a one-sentence diff (copy/doc tweak, one-line fix, config change, rename,
   missing guard): **skip spec + plan** (no `/spec-init`, no `/verify-claims`). Do not skip
   *thinking* — if the fix rests on a factual claim, quote the bytes inline before editing. Then
   **branch → fix → gate → ship** (finish through `/lavigie:ship`). Record nothing by hand that
   git already records.

   **Take the FULL path (steps 5–6) whenever ANY holds — never fast-path these:** the change
   touches migrations, auth, security, money/billing, data-loss, or PTY/session lifecycle; the
   requirements are ambiguous or a genuine design decision is involved; it spans multiple files
   with behavioral change, or you can't describe it in one sentence. When in doubt, full path.
   If you took the fast path, skip to step 7.

5. **Run /spec-init** (full path) via the Skill tool with a concise 2–4 sentence problem
   statement distilled from the task — not a verbatim copy.

6. **Run /verify-claims** (full path) via the Skill tool to pressure-test every factual claim in
   the task description and spec before any code. Treat the task description as claims, not
   ground truth. If a load-bearing claim is false/unverifiable, update the spec (`/spec-update`)
   and re-plan before continuing.

7. **If launch instructions are non-empty**, begin working on them immediately — on the fast
   path after step 4, on the full path after steps 5–6. Don't ask for confirmation; just start.
