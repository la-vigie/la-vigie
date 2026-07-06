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
