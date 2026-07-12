// Stryker mutation testing — TypeScript/frontend pure-function core (TASK-143).
//
// SCOPE: the `mutate` allowlist below is the frontend pure-function core only —
// pure helpers/parsers/decision fns that our testing doctrine already targets.
// UI components (*.tsx), React hooks (src/hooks/*), the Tauri IPC wrapper
// (src/api.ts), and the Zustand store (src/store/index.ts) are deliberately
// EXCLUDED: they are thin glue we don't unit-test, and mutating them would
// drown the score in unkillable mutants. See docs/mutation-testing.md for the
// full scope rationale and the advisory→gating rollout.
//
// Run (full baseline):      npx stryker run
// Run (changed files only):  npx stryker run --since=main
/** @type {import('@stryker-mutator/api/core').PartialStrykerOptions} */
export default {
  testRunner: 'vitest',
  coverageAnalysis: 'perTest',
  // Pure-function core only — keep in sync with docs/mutation-testing.md.
  mutate: [
    'src/lib/combineInitialPrompts.ts',
    'src/lib/taskName.ts',
    'src/notify/format.ts',
    'src/notify/registry.ts',
    'src/sound/resolve.ts',
    'src/sound/safe-parse.ts',
    'src/sound/source.ts',
    'src/components/Diff/comments.ts',
    'src/components/Diff/diffHeader.ts',
    'src/components/Diff/sendToAgent.ts',
    'src/components/Prompts/insertAtCursor.ts',
    'src/components/Terminal/runState.ts',
    'src/components/Terminal/fileDrop.ts',
  ],
  reporters: ['clear-text', 'progress', 'html', 'json'],
  // Advisory only for now: do not fail the run on a low score. When the
  // baseline stabilizes we raise `break` to gate code-class auto-merge.
  thresholds: { high: 80, low: 60, break: null },
};
