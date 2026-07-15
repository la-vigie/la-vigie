---
name: rename
description: Rename the current La Vigie task (the worktree you're in) to a new title via La Vigie's HookBridge. Use when the task's title should reflect what's actually being worked on.
targets: ["*"]
---

# Rename this La Vigie task

Rename **this** task — the one whose worktree you're in — by POSTing a new title to
La Vigie's HookBridge. La Vigie injects `LAVIGIE_HOOK_PORT` and `LAVIGIE_TASK_ID`
(the durable key, TASK-151) into every agent it launches; the rename is keyed on the
task_id so it only affects your own task.

## Steps

1. Determine the new title from the user's request (or the work at hand). Use just
   the title — no ticket-ID prefix (La Vigie shows the ticket as a separate chip).

2. POST it (plain-text body) to the rename endpoint. Skip silently if the env vars
   are absent (you're not running under La Vigie):

   ```bash
   LAVIGIE_ID="${LAVIGIE_TASK_ID:-$LAVIGIE_AGENT_ID}"
   if [ -n "$LAVIGIE_HOOK_PORT" ] && [ -n "$LAVIGIE_ID" ]; then
     curl -s -X POST \
       "http://127.0.0.1:$LAVIGIE_HOOK_PORT/rename/$LAVIGIE_ID" \
       --data-binary "<new title>"
   fi
   ```

3. Interpret the response: **200** echoes the applied name (renamed); **404** means
   the id was unknown; **400** means the name was blank after trimming.
