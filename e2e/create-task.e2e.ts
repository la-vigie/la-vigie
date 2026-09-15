import { execFileSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import path from 'node:path'

// TASK-142 track (b): create-task -> sidebar row.
//
// The brief assumed create-task hard-requires the native OS folder-picker (like
// "Add repository" does). It doesn't: `NewTaskForm.runCreate` (src/components/
// Sidebar/Sidebar.tsx) calls `createTask(repo.id, title, ...)` directly — the
// worktree location is derived server-side from the repo + worktrees_root, and the
// form is plain text inputs. So track (b) CAN drive create-task fully through the
// WebView. The native-picker boundary only applies to *adding a repo*
// (`open({ directory: true })` in `handleAddRepository`), so we seed one repo via
// `window.__TAURI__.core.invoke('add_repo', ...)` in-page — the same Tauri command
// "Add repository" calls once its picker resolves — and never touch OS chrome.
//
// `browser.tauri.execute()` (the plugin's mock-aware wrapper) was tried first, but
// it hangs waiting on `window.__wdio_original_core__`, which nothing in this app or
// in tauri-plugin-wdio-webdriver@1.2.0 ever sets — that hook needs a frontend-side
// guest-js integration this app doesn't have. Plain `browser.execute()` against
// `window.__TAURI__` works instead, which needs `app.withGlobalTauri: true`.
//
// IMPORTANT: this test requires the debug bundle to have been built with that flag
// baked in — a normal `tauri build --debug --bundles app` does NOT set it (we
// deliberately did not touch tauri.conf.json's production config for this). Build
// with `npm run tauri:build:e2e` (= `tauri build --debug --bundles app --config
// '{"app":{"withGlobalTauri":true}}'`, a build-time-only override) before running
// this spec — a fresh checkout or the Task 5 10x runner must use that script, not
// plain `tauri:build`.
// Poll until the Tauri bridge is usable from the page. Uses browser.execute
// (not browser.tauri.execute, which needs a guest-js hook this app lacks).
async function waitForTauriBridge(): Promise<void> {
  await browser.waitUntil(
    async () =>
      await browser.execute(() => {
        const t = (window as any).__TAURI__
        return !!(t && t.core && typeof t.core.invoke === 'function')
      }),
    {
      timeout: 30000,
      timeoutMsg: 'Tauri bridge (window.__TAURI__.core.invoke) never became ready',
    },
  )
}

describe('TASK-142 track (b): create task -> sidebar row', () => {
  let repoPath: string

  before(async () => {
    // A throwaway git repo for add_repo to adopt. It needs one commit so
    // create_task's `git worktree add` has something to check out.
    repoPath = mkdtempSync(path.join(tmpdir(), 'lavigie-e2e-repo-'))
    execFileSync('git', ['init', '-b', 'main', repoPath])
    execFileSync('git', ['-C', repoPath, 'config', 'user.email', 'e2e@example.com'])
    execFileSync('git', ['-C', repoPath, 'config', 'user.name', 'e2e'])
    execFileSync('git', ['-C', repoPath, 'commit', '--allow-empty', '-m', 'seed'])

    // The app mounts (#root) and injects the Tauri bridge (window.__TAURI__.core)
    // asynchronously after the WebDriver session opens. Seeding via a raw
    // `core.invoke('add_repo')` before the bridge exists throws and fails the
    // run, so wait for both to be ready first. `browser.execute` is NOT one of
    // the service's focus-tracked commands, so this poll adds no per-command
    // focus-timeout overhead. (Condition-based wait, not a fixed sleep.)
    await $('#root').waitForExist({ timeout: 30000 })
    await waitForTauriBridge()

    await browser.execute(
      async (p) => await (window as any).__TAURI__.core.invoke('add_repo', { path: p }),
      repoPath,
    )

    // add_repo doesn't push a store event to the frontend (repos are only
    // fetched on mount / explicit refresh()), so reload to pick up the seed.
    await browser.execute(() => window.location.reload())
    await $('#root').waitForExist({ timeout: 30000 })
    // reload() re-races the bridge injection, so re-await it before the test
    // starts driving the freshly-mounted UI.
    await waitForTauriBridge()
  })

  after(() => {
    // Ten runs of the flake budget shouldn't leave ten+ scratch repos in $TMPDIR.
    rmSync(repoPath, { recursive: true, force: true })
  })

  it('shows the new task row in the sidebar after create-task', async () => {
    const title = `e2e task ${Date.now()}`
    const before = (await $$('.sidebar__task-title')).length

    // Only one repo is seeded, so the button is unambiguous.
    await $('.sidebar__new-task-button').waitForExist({ timeout: 10000 })
    await $('.sidebar__new-task-button').click()

    const titleField = await $('[aria-label="Task title"]')
    await titleField.waitForDisplayed()
    await titleField.setValue(title)
    await $('.new-task-modal button[type="submit"]').click()

    await browser.waitUntil(
      async () => (await $$('.sidebar__task-title')).length > before,
      { timeout: 15000, timeoutMsg: 'no new task row appeared' },
    )

    // Read text via a plain in-page query rather than mapping over the
    // WebdriverIO element array -- `ElementArray.map()` doesn't resolve to a
    // plain iterable, which trips up `Promise.all`.
    const titles = await browser.execute(() =>
      Array.from(document.querySelectorAll('.sidebar__task-title')).map((el) => el.textContent),
    )
    await expect(titles).toContain(title)
  })
})
