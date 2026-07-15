---
name: spec-init
description: Create a spec file for the current task to record thinking and decisions
targets: ["*"]
claudecode:
  allowed-tools: Bash, Read, Write, Glob
  argument-hint: '<problem statement summary>'
---

# Spec Init: Create a spec file for this task

You are creating a local spec/decision log for the current task.

## Instructions

1. **Determine the task ID** from the current git branch name or worktree directory.

2. **Create the spec file** in the project's memory directory at:
   ```
   memory/spec_<TASK_ID>.md
   ```

   Use this format:

   ```markdown
   ---
   name: Spec for <TASK_ID>
   description: Decision log and spec for task <TASK_ID>
   type: project
   ---

   # <TASK_ID> — Spec & Decision Log

   ## Problem Statement

   $ARGUMENTS

   ## Decisions

   <!-- Use /spec-update to append decisions here -->
   ```

3. **Update MEMORY.md** — add a line:
   ```
   - [Spec <TASK_ID>](spec_<TASK_ID>.md) — spec and decision log
   ```

4. Confirm the spec file was created.
