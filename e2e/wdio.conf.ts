import { mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'

// TASK-142 track (b) launch spike. The @wdio/tauri-service embedded provider runs a
// W3C WebDriver server inside the app (registered debug-only in src-tauri/src/lib.rs),
// so no external tauri-driver / CrabNebula subscription is needed on macOS.
//
// The service `spawn`s the given path directly (it does NOT resolve from tauri.conf.json,
// and passing the .app directory yields EACCES). So point `application` at the actual
// executable *inside* the debug bundle produced by `tauri build --debug` — it still runs
// within the .app bundle context (Info.plist / identifier). Cargo package is `vigie`;
// the bundle/productName is "La Vigie".
const APP_BUNDLE = path.resolve(
  process.cwd(),
  'src-tauri/target/debug/bundle/macos/La Vigie.app/Contents/MacOS/vigie',
)

// Isolate every e2e run from the developer's real La Vigie data. `app_data_dir()`
// (src-tauri/src/lib.rs) resolves via the `dirs` crate, which on macOS is
// `$HOME/Library/Application Support/<bundle id>` — and `dirs` reads `$HOME` from
// the process environment (dirs-sys `home_dir()` checks `env::var_os("HOME")` before
// falling back to getpwuid), not a syscall the sandbox could shadow another way. The
// embedded provider spawns the app binary with `{...process.env, ...options.env}`
// (see `startEmbeddedDriver` in @wdio/tauri-service), so overriding just `HOME` here
// redirects vigie.db + worktrees_root + sounds + concierge into a scratch dir for the
// whole run, leaving PATH etc. untouched. Without this, a spec that creates a repo/task
// (e.g. create-task.e2e.ts) would write into the developer's live
// ~/Library/Application Support/com.lavigie/vigie.db and worktrees — a real repo/task
// would appear in their actual running La Vigie window.
const SCRATCH_HOME = mkdtempSync(path.join(tmpdir(), 'lavigie-e2e-home-'))

export const config: WebdriverIO.Config = {
  runner: 'local',
  // Resolved relative to THIS config file's directory (e2e/), so no `e2e/` prefix.
  specs: ['./*.e2e.ts'],
  maxInstances: 1,
  capabilities: [
    {
      browserName: 'tauri',
      'tauri:options': { application: APP_BUNDLE },
      'wdio:tauriServiceOptions': { env: { HOME: SCRATCH_HOME } },
    },
  ],
  services: [['@wdio/tauri-service', { driverProvider: 'embedded' }]],
  framework: 'mocha',
  reporters: ['spec'],
  logLevel: 'info',
  // Per-test timeout. TASK-142 needed 300s here only to tolerate the WebDriver
  // focus-poll tax: @wdio/tauri-service runs `ensureActiveWindowFocus` before
  // every findElement/$/$$/click, whose guest script polled the hardcoded 5s for
  // `window.__wdio_original_core__` — a hook this app never exposed — so ~24
  // commands cost ~120-145s of pure dead-wait. TASK-245 removed the tax by
  // aliasing the global Tauri core onto that hook in the e2e build
  // (src/e2e/exposeWdioCore.ts), so the poll resolves instantly; the create-task
  // flow now runs in ~3-5s. 60s is a sane ceiling with generous headroom.
  mochaOpts: { timeout: 60000 },
}
