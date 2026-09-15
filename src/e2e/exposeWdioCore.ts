// Kills the ~5s/command WebDriver focus-poll tax in the GUI-verification e2e.
//
// @wdio/tauri-service runs `ensureActiveWindowFocus` before every focus-tracked
// command (findElement/$/$$/click/getTitle). That calls `getWindowStates` via the
// service's direct-eval channel, whose wrapped guest script does:
//
//   while (!window.__wdio_original_core__?.invoke && (Date.now()-start) < 5000) sleep(50)
//
// i.e. it polls the *hardcoded 5s* for a guest-js hook, `window.__wdio_original_core__`,
// that this app never exposes (we have no wdio guest-js integration). With ~24 commands
// per create-task flow that's ~120s of pure dead-wait, dominating the ~170s runtime.
//
// Fix: alias the already-global Tauri core onto that hook name. Once
// `window.__wdio_original_core__.invoke` exists, the poll resolves instantly; the
// subsequent `core.invoke('plugin:wdio|get_window_states')` then rejects fast (the
// tauri-plugin-wdio-webdriver plugin registers as `wdio-webdriver` and exposes no such
// IPC command — it's an HTTP server), so `getWindowStates` returns `[]` and focus is
// skipped. Net: a few ms per command instead of 5s.
//
// This is strictly e2e-only. `window.__TAURI__.core` is present ONLY when the build sets
// `withGlobalTauri` — which just `tauri:build:e2e` does; production `tauri.conf.json`
// never enables it. And the whole install is gated behind the `VITE_E2E` build flag
// (set by `tauri:build:e2e`), so it is dead-code-eliminated from the production bundle.
// Even when it does run, it only aliases an object `withGlobalTauri` already exposed
// globally — no new capability surface.

interface WdioCoreWindow {
  __TAURI__?: { core?: { invoke?: unknown } }
  __wdio_original_core__?: unknown
}

/**
 * Alias `window.__TAURI__.core` onto `window.__wdio_original_core__` when the former
 * exists (e2e build's `withGlobalTauri`) and the latter isn't set yet.
 *
 * Pure and idempotent: returns `true` once the hook is (or already was) in place,
 * `false` when the Tauri global isn't available yet (production, or before injection).
 */
export function exposeWdioOriginalCore(win: WdioCoreWindow): boolean {
  if (win.__wdio_original_core__) {
    return true
  }
  const core = win.__TAURI__?.core
  if (core && typeof core.invoke === "function") {
    win.__wdio_original_core__ = core
    return true
  }
  return false
}

/**
 * Install the hook, retrying on a bounded interval because `window.__TAURI__` is
 * injected asynchronously after page load (the create-task spec polls a bridge-ready
 * gate for the same reason). Stops as soon as the alias is in place, or after the
 * budget elapses. Bounded so it can never spin forever if the global never appears.
 */
export function installWdioCoreHook(
  win: WdioCoreWindow = window as unknown as WdioCoreWindow,
  { intervalMs = 50, maxWaitMs = 10000 }: { intervalMs?: number; maxWaitMs?: number } = {},
): void {
  if (exposeWdioOriginalCore(win)) {
    return
  }
  const maxAttempts = Math.ceil(maxWaitMs / intervalMs)
  let attempts = 0
  const timer = setInterval(() => {
    attempts += 1
    if (exposeWdioOriginalCore(win) || attempts >= maxAttempts) {
      clearInterval(timer)
    }
  }, intervalMs)
}
