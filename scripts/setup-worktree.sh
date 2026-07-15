#!/usr/bin/env bash
# Prepare a freshly-created worktree for development.
#
# Run automatically by La Vigie after `git worktree add`, or by hand:
#   ./scripts/setup-worktree.sh
#
# Keep this idempotent and cheap-when-up-to-date: it may run on every new
# worktree. It installs JS deps and warms the Rust build so the first
# `tauri dev` isn't a cold compile.
set -euo pipefail

# Resolve repo root regardless of where this is invoked from.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "==> Installing npm dependencies in $ROOT"
# Prefer `npm ci`: it installs strictly from the committed lockfile and never
# rewrites it, so a fresh worktree can't start life with a phantom
# `package-lock.json` diff (some npm versions, e.g. 11.4–11.5, rewrite the lock
# on a non-Linux host by stripping `libc` from optional Linux deps — TASK-208).
# Fall back to `npm install` when the lockfile is out of sync with package.json
# (npm ci errors hard in that case), where updating the lock is the intent.
npm ci || npm install

# Warm the Rust build so the first `tauri dev` isn't a cold compile.
# Best-effort: a missing Rust toolchain shouldn't fail worktree setup.
if [ -d src-tauri ]; then
  # Pick up rustup's env if cargo isn't already on PATH (e.g. GUI-launched).
  if ! command -v cargo >/dev/null 2>&1 && [ -f "$HOME/.cargo/env" ]; then
    # shellcheck disable=SC1091
    . "$HOME/.cargo/env"
  fi
  if command -v cargo >/dev/null 2>&1; then
    echo "==> Warming the Rust build (cargo build)"
    (cd src-tauri && cargo build) || echo "!! cargo build failed; skipping warm-up"
  else
    echo "!! cargo not found; skipping Rust warm-up"
  fi
fi

echo "==> Worktree ready. Run 'npm run tauri:dev' to start the app."
