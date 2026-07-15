#!/usr/bin/env bash
#
# bump-version.sh — set the La Vigie app version across every version-bearing
# file in one shot, so a release can't ship with a stale version again.
#
# Rewrites the app version ONLY (never a dependency that happens to share a
# version number):
#   - package.json                "version"
#   - package-lock.json           root "version" + packages[""].version  (via npm)
#   - src-tauri/tauri.conf.json   top-level "version"
#   - src-tauri/Cargo.toml        [package] version
#   - src-tauri/Cargo.lock        the `vigie` crate entry
#
# It edits files only — it does NOT commit, tag, or push. Commit the result
# yourself (or let the release flow do it). Idempotent: re-running with the
# same version is a no-op.
#
# Usage:
#   ./scripts/bump-version.sh 0.3.0
#   ./scripts/bump-version.sh v0.3.0     # a leading "v" is stripped

set -euo pipefail

# --- args --------------------------------------------------------------------
VERSION="${1:-}"
if [[ -z "$VERSION" ]]; then
  echo "usage: $(basename "$0") <version>   (e.g. 0.3.0 or v0.3.0)" >&2
  exit 1
fi
VERSION="${VERSION#v}"
if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]]; then
  echo "error: '$VERSION' is not a valid semver version (e.g. 0.3.0)" >&2
  exit 1
fi

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

echo "Bumping La Vigie app version to $VERSION ..."

# --- JS: package.json + package-lock.json (both self-version entries) ---------
# npm updates the manifest and the lockfile's root + packages[""] version and
# touches nothing else. --allow-same-version keeps a re-run a no-op; there is no
# version/preversion/postversion lifecycle script in package.json to fire.
npm version "$VERSION" --no-git-tag-version --allow-same-version >/dev/null
echo "  package.json + package-lock.json  -> $VERSION"

# --- src-tauri/Cargo.toml: only the [package] version, never a dependency's ---
tmp="$(mktemp)"
awk -v v="$VERSION" '
  /^\[/ { inpkg = ($0 == "[package]") }
  inpkg && !done && /^version[[:space:]]*=/ {
    sub(/"[^"]*"/, "\"" v "\""); done = 1
  }
  { print }
' src-tauri/Cargo.toml > "$tmp" && mv "$tmp" src-tauri/Cargo.toml
echo "  src-tauri/Cargo.toml              -> $VERSION"

# --- src-tauri/tauri.conf.json: the top-level "version" -----------------------
tmp="$(mktemp)"
awk -v v="$VERSION" '
  !done && /^[[:space:]]*"version"[[:space:]]*:/ {
    sub(/"version"[[:space:]]*:[[:space:]]*"[^"]*"/, "\"version\": \"" v "\""); done = 1
  }
  { print }
' src-tauri/tauri.conf.json > "$tmp" && mv "$tmp" src-tauri/tauri.conf.json
echo "  src-tauri/tauri.conf.json         -> $VERSION"

# --- src-tauri/Cargo.lock: the `vigie` crate entry ----------------------------
# Each [[package]] block lists name then version; bump the version line that
# belongs to the `vigie` package only.
tmp="$(mktemp)"
awk -v v="$VERSION" '
  /^\[\[package\]\]/ { pkg = 0 }
  $0 == "name = \"vigie\"" { pkg = 1 }
  pkg && /^version[[:space:]]*=/ {
    sub(/"[^"]*"/, "\"" v "\""); pkg = 0
  }
  { print }
' src-tauri/Cargo.lock > "$tmp" && mv "$tmp" src-tauri/Cargo.lock
echo "  src-tauri/Cargo.lock (vigie)      -> $VERSION"

echo "Done. Review with: git diff"
