# Mutation testing (TASK-143)

We have hundreds of green tests and, until now, no idea whether they actually
*kill mutants*. Line coverage is a vanity metric — the field's number for
LLM-written test suites is **~4% mutation score at 100% line coverage**
(HumanEval-Java study; see `docs/research/2026-07-06-sdlc-watch/`). Mutation
testing injects small faults ("mutants") into the code and checks that some test
fails. A surviving mutant is a hole in the suite.

This doc records **what we mutate**, **what we deliberately don't**, **how to
run it**, and the **advisory → gating rollout**.

- **Rust:** [`cargo-mutants`](https://mutants.rs) — config in `src-tauri/.cargo/mutants.toml`.
- **TypeScript/frontend:** [Stryker](https://stryker-mutator.io) — config in `stryker.config.mjs`.

## Scope: the pure-function core only

We mutate the **pure-function core** our testing doctrine already targets —
parsers, helpers, migrations, and decision/mapping functions. We do **not**
mutate the thin Tauri / `gh` / `git` / PTY glue we deliberately don't unit-test
(per `CLAUDE.md`). A repo-wide run would drown the score in unkillable mutants
in glue that has no unit tests by design, telling us nothing about test quality.

Scope is enforced as an **allowlist** in each tool's config — adding a new pure
module means adding it to the list, so scope creep is explicit and reviewable.

### Rust — in scope (`examine_globs`)

| File | Why it's pure-core |
|------|--------------------|
| `github/mod.rs` | PR JSON parsers (`parse_pr_status`, `parse_comments`, `normalize_check`) |
| `agent/status.rs` | agent-status state machine / mapping |
| `agent/spec.rs` | agent spec parsing |
| `agent/models.rs` | agent model helpers |
| `remote/auth.rs` | token auth comparison |
| `remote/tailscale.rs` | `tailscale`-output parsing (CLI-invoking wrappers excluded) |
| `claude_path.rs` | `PATH` split (pure); filesystem probes excluded |
| `shell_env.rs` | login-shell env parsing (shell-spawning fns excluded) |
| `docs.rs` | docs helpers |
| `session/mod.rs` | session helpers |
| `sound/mod.rs` | sound-resolution decision fns |
| `store/mod.rs` | SQLite migrations, `slugify`, pure CRUD (tested against an in-memory DB) |

**Function-level exclusions inside otherwise-pure files** (`exclude_re`): some
allowlisted modules mix pure parsers with IO glue on the same file. We keep the
parsers in scope and skip the glue functions by name — otherwise their
inevitably-surviving mutants would depress the pure-core score:

- `github/mod.rs` — async `gh` CLI wrappers: `gh_status`, `pr_status`,
  `pr_comments`, `create_pr`, `merge_pr`, `run_gh`.
- `remote/tailscale.rs` — `tailscale` CLI callers: `find_tailscale`, `run`,
  `magic_dns_name`, `serve_start`, `serve_reset`, `serve_funnel_active`.
- `shell_env.rs` — `capture_login_env` (spawns a login shell), `hydrate`.
- `claude_path.rs` — `find_binary`, `is_executable_file` (filesystem probes);
  the pure `split_path` stays in scope.

> The `exclude_re` patterns are **word-anchored** (`\bpr_status\b`) on purpose:
> a bare `pr_status` would also match — and wrongly skip — the pure
> `parse_pr_status` parser we specifically want mutated.

### Rust — deliberately excluded (glue)

`git/mod.rs` (git CLI wrapper — 43 async fns, no production pure fns),
`hooks/mod.rs` (axum HookBridge server), `setup/mod.rs` (filesystem setup),
`meeting/mod.rs` (native macOS CoreAudio/AVFoundation camera/mic probe — FFI to
real hardware, untestable headlessly; same category as the TASK-105 automute
probe), `commands.rs` + `*_commands.rs` (Tauri command glue), the MCP server
(`mcp/mod.rs`), `remote/{server,commands,mod}.rs`, `concierge/mod.rs`,
`launch.rs`, `agent/mod.rs` (PTY supervisor), `state.rs`, `lib.rs`, `main.rs`.

> Note: `git/mod.rs` and `mcp/mod.rs` *do* contain some pure parsing/shaping
> functions with tests. They're excluded for the v1 baseline to keep the score
> clean; promoting their pure fns (via function-level `examine_re`) is a
> phase-2 candidate once the baseline is understood.

### Frontend — in scope (`mutate`)

All pure, all with test siblings: `lib/combineInitialPrompts.ts`,
`lib/taskName.ts`, `notify/format.ts`, `notify/registry.ts`, `sound/resolve.ts`,
`sound/safe-parse.ts`, `sound/source.ts`, `components/Diff/comments.ts`,
`components/Diff/diffHeader.ts`, `components/Diff/sendToAgent.ts`,
`components/Prompts/insertAtCursor.ts`, `components/Terminal/runState.ts`,
`components/Terminal/fileDrop.ts`.

### Frontend — deliberately excluded (glue)

UI components (`*.tsx`), React hooks (`src/hooks/*`), the Tauri IPC wrapper
(`src/api.ts`), and the Zustand store (`src/store/index.ts`). The store has
well-tested pure reducer logic and is a phase-2 candidate, but for v1 it stays
out to keep the pure-core score honest.

## How to run

### Rust (`cargo-mutants`)

```sh
cd src-tauri

# Full baseline over the pure-function core allowlist (--in-place: no target/ copy)
cargo mutants --in-place

# Fast scope check — list files/mutants without running tests
cargo mutants --list-files
cargo mutants --list

# Changed lines only (day-to-day / CI on a PR) — mutate only what the diff touches.
# --relative makes the diff's paths crate-relative (src/…), which --in-diff matches;
# `-- src` (not 'src/**/*.rs', whose ** git treats as * and skips top-level src/*.rs).
git diff --relative origin/main...HEAD -- src > /tmp/pr.diff
cargo mutants --in-diff /tmp/pr.diff
```

Results land in `src-tauri/mutants.out/` (git-ignored). A **MISSED** mutant =
a surviving fault = a test-suite hole; **CAUGHT** = a test killed it;
**UNVIABLE** = the mutant didn't compile (not counted against the score).

### Frontend (Stryker)

```sh
# Full baseline over the pure-core allowlist
npx stryker run

# Changed files only (day-to-day / CI on a PR)
npx stryker run --since=main
```

The HTML report lands in `reports/mutation/` (git-ignored).

## Baseline mutation scores

_Recorded on the TASK-143 branch (2026-07-08). Advisory only — see rollout below._

| Suite | Scored mutants | Killed | Survived | No-cov | Unviable | Score |
|-------|--------:|-------:|---------:|-------:|--------:|------:|
| Rust (cargo-mutants) | 200 | 187 | 13 | — | 30 | **93.5%** |
| Frontend (Stryker) | 317 | 257 | 45 | 15 | — | **81.07%** |

Score = (killed + timeout) / (killed + timeout + survived + no-coverage) — a
timeout means the mutant hung a test, i.e. it was detected. `Unviable` mutants
(didn't compile) are not scored. Both baselines had 0 timeouts, so this equals
killed / (killed + survived + no-coverage) here. Both are **~20× the field's 4% baseline** for
LLM-written suites — the pure-function core genuinely bears weight.

> These are the *starting* numbers, not a target. The point of the first run is
> to learn the baseline, then decide where to set a gate.

**Rust read (2026-07-08):** 93.5% (13 survivors out of 200). With the glue
scoped out, every survivor is a genuine pure-logic test hole worth closing:

- `github/mod.rs::normalize_check` (4) — the check-status classifier's match
  arms (`FAILURE|TIMED_OUT|…`, `NEUTRAL|SKIPPED|…`, `""`, `FAILURE|ERROR`) can
  be deleted without a test failing → conclusions aren't pinned per bucket.
- `claude_path.rs::split_path` (3) — the pure `PATH` splitter can return `[]`
  and drop its `!` without detection.
- `docs.rs::resolve_docs` (2) — `||` → `&&` in the resolution condition.
- `sound/mod.rs` (3) — two `*`→`+` in a size/const expression (L13) and a
  `>`→`>=` boundary in `import_sound`.
- `store/mod.rs::set_task_hidden` (1) — the setter's effect isn't asserted
  (1 survivor across 51 store tests — excellent).

**Frontend read (2026-07-08):** 81.07% (45 survivors + 15 no-coverage out of
317 scored). Holes worth closing first: `components/Diff/sendToAgent.ts` (66%),
`sound/source.ts` (49%, mostly `NoCoverage` on MIME-type map entries),
`components/Terminal/fileDrop.ts` (70%), `notify/registry.ts` (67%). Several
files are already at 100% (`comments.ts`, `diffHeader.ts`, `insertAtCursor.ts`,
`combineInitialPrompts.ts`, `taskName.ts`, `format.ts`).

> **Running cost note:** the full Rust baseline is ~200 mutants × (build + test)
> ≈ 18 min serial. Run it **`--in-place`** (mutates the tree directly; no
> per-worker `target/` copy) — the parallel copy mode spawns one multi-GB
> `target/` clone *per job* (≈17 GB at `-j 6`) which pins the CPU and then makes
> Spotlight re-index gigabytes of build artifacts. For day-to-day use, the
> diff-scoped run (below) is seconds-to-minutes. CI runs `--in-place` too on a
> throwaway runner (only the PR diff's mutants are built).

## Rollout: advisory → gating

**Now (advisory).** Neither tool fails a build. `stryker.config.mjs` sets
`thresholds.break: null`; cargo-mutants is run but its exit code is not wired to
block anything. We run it on changed files and read the number; a drop is a
signal to add tests, not a merge blocker.

**Later (gating), only when all of these hold:**

1. The baseline has been stable for **a clean month** and we understand which
   surviving mutants are real gaps vs. acceptable (e.g. equivalent mutants).
2. A **minimum mutation score on changed code** is agreed — proposed starting
   gate: **no *new* MISSED mutants introduced by the diff** (a delta gate, not
   an absolute-score gate), which avoids penalizing pre-existing debt.
3. The gate applies to **code-class auto-merge only** (per `MERGE_POLICY.md`).
   **Docs-class** auto-merge is exempt. **Healed/test-only** changes never
   auto-qualify regardless of score — those always go to a human queue.

Until (1)–(3) are met it stays advisory. This mirrors the cross-vendor review
gate rollout (TASK-138) and the GUI-verification pilot (TASK-142): prove the
signal before letting it block a merge.

## Integration point

**Decision: an advisory CI job on changed files**, matching the existing
`.github/workflows/{merge-policy,pr-review}.yml` pattern — not a pre-`/ship`
local hook (mutation runs are too slow for the local hot path). The workflow
lives at `.github/workflows/mutation-advisory.yml`; it runs both tools scoped to
the PR diff, prints the scores, and **does not block** (advisory). When the
rollout criteria above are met, flipping it to gating means feeding a
`review-gate-mutation` status check into `MERGE_POLICY.md` — the same mechanism
TASK-138 uses for the Mistral reviewer.
