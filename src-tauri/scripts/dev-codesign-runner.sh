#!/bin/sh
# Cargo runner (wired in ../.cargo/config.toml) used in place of directly executing the
# dev binary for `cargo run` / `tauri dev` on macOS. It re-signs the freshly linked
# binary with a stable code-signing identity, then execs it.
#
# Why: `tauri dev` re-links target/debug/vigie on every change and Tauri does not sign
# dev binaries. Without a stable identity, macOS keys TCC grants (Screen Recording,
# Accessibility, ...) to the binary's cdhash, so every rebuild silently revokes them.
# Signing here keeps the designated requirement (identifier com.lavigie + the cert)
# stable across rebuilds, matching the `tauri build` .app so both share ONE TCC grant.
#
# Safety: this is a no-op when the identity is not in the keychain (CI, other checkouts,
# contributors without the cert) and NEVER aborts the run — so it can't break
# `cargo run` / `cargo test`. Override the identity with LAVIGIE_DEV_SIGNING_IDENTITY.
set -u

BIN="$1"
shift

# Selector for the signing cert. "Apple Development" matches a developer's local
# Apple Development cert without hardcoding anyone's personal identity in the repo.
# Override with a more specific string (or a SHA-1) via LAVIGIE_DEV_SIGNING_IDENTITY
# if you have more than one matching cert.
IDENTITY="${LAVIGIE_DEV_SIGNING_IDENTITY:-Apple Development}"

# Only sign if that identity is actually available; otherwise run unsigned, as before.
if security find-identity -v -p codesigning 2>/dev/null | grep -qF "$IDENTITY"; then
  if ! codesign --force --sign "$IDENTITY" -i com.lavigie "$BIN" >/dev/null 2>&1; then
    echo "dev-codesign-runner: warning: codesign failed for $BIN (running unsigned)" >&2
  fi
fi

exec "$BIN" "$@"
