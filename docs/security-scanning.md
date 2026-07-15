# Security scanning

Automated security scanning for this repo (TASK-71). Two complementary layers:

## 1. Weekly advisory scan — `.github/workflows/security-scan.yml`

A scheduled GitHub Actions workflow (Mondays 07:00 UTC, plus manual
`workflow_dispatch`) runs three scanners over the repo:

| Surface        | Tool                          | Notes                                            |
| -------------- | ----------------------------- | ------------------------------------------------ |
| Rust crates    | `cargo audit`                 | RustSec advisories vs `src-tauri/Cargo.lock`     |
| npm packages   | `npm audit --package-lock-only` | npm registry advisories vs root `package-lock.json` |
| Secrets        | `gitleaks` (pinned binary)    | Full git history, `--redact`                     |

On any finding it opens (or updates) a single deduplicated GitHub issue labelled
`security-scan`; when a later run is clean it closes that issue. It is **not** a
PR gate — it posts no `review-gate-*` status and does not affect `MERGE_POLICY.md`.

The finding→issue decision logic is the unit-tested
`scripts/security-scan-report.mjs`; the workflow YAML, `scripts/security-scan.mjs`,
and the `gh` calls are thin, unit-tested-exempt glue.

## 2. Continuous dependency updates — `.github/dependabot.yml`

Weekly Dependabot version-update PRs for `cargo` (`/src-tauri`), `npm` (`/`), and
`github-actions` (`/`).

> **Repo setting:** enable Dependabot **security alerts / security updates** under
> Settings → Code security. `dependabot.yml` configures version-update PRs only.

## Decisions (why these tools / this runner)

- **Runner = GitHub Actions cron**, not n8n or local cron: free, no new infra,
  matches every other workflow in the repo.
- **Scanners = ecosystem-native** (`cargo audit` + `npm audit`) + `gitleaks`,
  meeting the acceptance criteria (Rust advisories + npm advisories + secret
  detection) with no paid tooling.
- **No CodeQL / native GitHub secret scanning:** this is a private repo, where
  both require paid GitHub Advanced Security.
- **gitleaks binary, not `gitleaks-action`:** the action is license-gated for
  organisation-owned repos; the MIT-licensed binary is not.
- **Notification = a deduplicated GitHub issue**, so a finding is tracked and
  visible rather than lost in a red scheduled run.

## Known edge cases

- An unfixable RustSec advisory keeps the issue open until an
  `--ignore RUSTSEC-XXXX` (checked-in `audit.toml`) entry is added — deliberate,
  for persistent visibility.
- If gitleaks flags a test fixture, add a `.gitleaks.toml` allowlist entry.
