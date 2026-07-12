---
name: scan-for-doc-change
description: Use during /ship (or manually) to check whether the branch diff adds user-facing functionality that should be documented before it ships. Emits an advisory plus a drafted doc entry. Advisory only — never blocks.
---

# Scan for Doc Change

Detect whether the current branch introduces **user-facing** functionality that isn't yet
reflected in the docs, and — if so — surface an advisory naming the right doc target plus a
drafted entry covering how to **use** and where to **extend** the feature.

This is **advisory**. It never blocks a ship. Internal-only diffs produce a clean "nothing to
document" result and no nag.

## When this runs

- Invoked by `/ship` after the review gate passes (if the repo defines this skill).
- Manually, any time: `/scan-for-doc-change`.

## Steps

1. **Determine the base branch and read the diff.**
   ```bash
   BASE="$(gh repo view --json defaultBranchRef -q .defaultBranchRef.name 2>/dev/null || echo main)"
   git diff "origin/$BASE...HEAD" --stat || git diff "$BASE...HEAD" --stat
   git diff "origin/$BASE...HEAD"       || git diff "$BASE...HEAD"
   ```

2. **Classify the change as user-facing or internal.** A change is **user-facing** if it adds or
   alters a surface a *user of La Vigie* (or a downstream integrator) can see or call:
   - a new `#[tauri::command]` / IPC command exposed to the frontend (`src-tauri/src/commands.rs`);
   - a new La Vigie **MCP tool** (the MCP server agents call) or a new **HookBridge HTTP route**
     (`src-tauri/src/hooks/`);
   - a new **setting / flag / repo option / agent engine** a user can choose;
   - a new **UI feature, pane, or control** (a new component or a visible capability in `src/`);
   - an edit that makes an **already-documented** feature drift from README / `docs/getting-started.md`.

   A change is **internal** (→ stay silent) if it is only: a refactor, a pure parser/helper/
   migration change, tests, CI/workflow, build config, or thin glue with no new user-visible or
   caller-visible surface. When unsure, lean **internal** for a one-line / low-blast change and
   **user-facing** when a new named capability crosses an IPC / MCP / HTTP / UI boundary.

3. **Check whether it's already documented.** Grep the doc surfaces for the feature's name/keywords:
   ```bash
   grep -rin "<feature keyword>" README.md docs/ CONTRIBUTING.md
   ```
   `documented` is true if the diff already updates the relevant doc, or the feature is clearly
   described there.

4. **Pick the doc target(s)** for each user-facing surface:
   - **Use** — README `## Features` (the user-facing catalog) and/or `docs/getting-started.md`
     (when the feature fits the core create → run → review → steer flow).
   - **Extend** — `CONTRIBUTING.md` / the architecture overview: name the extension seam (the
     `commands.rs` glue, the `hooks/` route table, the agent-engine registry, the MCP tool list)
     so a contributor knows where to add the next one.

5. **Emit the result** — a short human advisory, then exactly one machine-readable line:
   ```
   doc-scan: {"userFacing": <true|false>, "surfaces": ["<kind>", ...], "targets": ["README#features", "CONTRIBUTING.md"], "documented": <true|false>}
   ```
   - `userFacing: false` → advisory is one line: `No user-facing changes detected — nothing to document.` with `surfaces: []`, `targets: []`.
   - `userFacing: true` and `documented: false` → advise the target(s) and include a **drafted entry** (below).
   - `userFacing: true` and already `documented: true` → say so; no draft needed.

## Drafted entry format

For each undocumented user-facing surface, draft a concrete entry the caller can drop into the
target. Cover both halves:

- **Use:** one or two sentences on what it is and how a user invokes or sees it.
- **Extend:** one sentence pointing at the seam where a contributor adds the next one.

Example (a new MCP tool):
> **`finish_task` (MCP tool)** — lets an agent tear down its own task (stop the PTY, remove the
> worktree, delete the row) by id; defaults to the caller's task. *Use:* call it from an agent
> session or the concierge. *Extend:* MCP tools are registered in the La Vigie MCP server; add a
> tool there and route it through the shared teardown core (`teardown.rs`).

## Output contract (for /ship)

`/ship` reads the `doc-scan:` line and NEVER blocks on this skill. When `userFacing && !documented`,
`/ship` surfaces the advisory and offers to write the drafted entry into the branch (same PR),
proceeding if declined.
