# Security Policy

## Reporting a vulnerability

Please **do not** open a public issue for security problems. Instead, report privately via GitHub's
[private vulnerability reporting](https://docs.github.com/en/code-security/security-advisories/guidance-on-reporting-and-writing-information-about-vulnerabilities/privately-reporting-a-security-vulnerability):
go to the repository's **Security** tab → **Report a vulnerability**. You can expect an initial
response within a reasonable time frame, and we'll coordinate a fix and disclosure with you.

## Scope & threat model

La Vigie is a **local desktop application**. It has no hosted backend and stores no cloud
credentials. Notable trust boundaries:

- **Loopback hook server** — the app runs a small HTTP server bound to `127.0.0.1` on an ephemeral
  port to receive agent lifecycle hook callbacks. It is not exposed off the local machine.
- **`git` / `gh` invocation** — all Git and GitHub operations shell out to the `git` and `gh` CLIs
  using argument vectors (never a shell string), so values like task titles and PR bodies cannot be
  used for command injection. GitHub access relies entirely on your existing `gh` authentication; the
  app stores no tokens.
- **Agent PTYs** — agents run as local child processes in their task's worktree with your user's
  permissions. Only run agents and repositories you trust.

If your project enables any optional remote-access feature, treat the pairing/token material as
sensitive and only expose it over networks you trust.

## Supported versions

This project is under active development; security fixes target the latest `main`. Please make sure
you're on the most recent release before reporting.

## Known accepted risks

Advisories the scheduled security scan (`security-scan.yml`, TASK-71) will keep re-flagging because
no viable fix exists yet, tracked here so they aren't repeatedly re-investigated from scratch:

- **`deepmerge-ts` <8.0.0 (GHSA-ggr8-5vv4-36mx, high)** and its transitive sibling **`extract-zip` \*
  (GHSA-jmr9-qjv8-65gv / GHSA-7pqw-9j4j-h8q3, high)** — both reachable only through
  `@wdio/tauri-service`'s own pinned `webdriverio@9.30.x`, which hasn't yet picked up the
  `@wdio/config`/`@wdio/utils` bump to `deepmerge-ts@^8.0.0` (that fix landed only in
  `@wdio/config@9.31.x`+, upstream at commit history for `webdriverio/desktop-mobile`). Confirmed as
  of 2026-09-15 that even `@wdio/tauri-service`'s `next` prerelease still pins `webdriverio@9.30.0`,
  and `npm audit fix --force` does not fix it either — it can only *downgrade* the whole `@wdio/*`
  tree to 8.14.x (which also regresses `deepmerge-ts` itself to 5.1.0, i.e. more exposed, not less)
  and npm reports "No fix available for @wdio/tauri-service@*" even after that downgrade.
  - **Reach:** dev-only. `@wdio/*` and `@wdio/tauri-service` are `devDependencies` used solely by the
    WebDriver e2e harness (TASK-142); they are never bundled into the shipped Tauri app, so this does
    not reach production or any end user.
  - **Re-check:** re-run `npm audit fix --dry-run` after any `@wdio/tauri-service` version bump — once
    it repins to `webdriverio@>=9.31.x`, this resolves on its own via `npm audit fix` (no `--force`,
    no `package.json` range change needed; `@wdio/tauri-service` is already declared as `^1.2.0`).
