# Code signing (macOS) — build options & stable TCC grants

## Two build options

`tauri build` on macOS can sign the `.app` two ways. Pick per what you need:

| Command | Signing | Needs an Apple cert? | TCC grants (Screen Recording, …) |
|---------|---------|----------------------|----------------------------------|
| `npm run tauri:build` | **Apple Development** identity | **yes** | **persist across rebuilds** (keyed to the stable identity) |
| `npm run tauri:build:unsigned` | ad-hoc (`-`) | no | **revoked on every rebuild** (new `cdhash` each time) |

(Tauri applies the **hardened runtime** to *both* variants, so both need the vendored-OpenSSL
fix below — the choice of identity only changes cert requirement and TCC-grant stability.)

**Default (`tauri:build`) is signed** — it's what you want for day-to-day dev on a machine
that has an Apple Development cert, because privacy grants survive rebuilds (see below). The
`unsigned` variant is for **contributors without a cert** (or CI-like one-off builds): it can't
persist TCC grants, but it needs no Apple identity and never fails for lack of one. Both produce
a runnable `.app`; on Apple silicon even the "unsigned" build is ad-hoc signed (macOS requires
at least that to launch).

> **Why signed is the default and not, say, always ad-hoc:** the signed build is the one that
> matches the `tauri dev` binary's identity, so a single Screen Recording grant covers both.

## OpenSSL must be statically linked (or the signed build crashes at launch)

`webauthn-rs` (passkey auth, TASK-240) pulls in `openssl`, which **by default dynamically links
the host's OpenSSL** — on macOS, Homebrew's `libssl.3.dylib`. Tauri signs the `.app` with a
**hardened runtime** (both build variants), whose **library validation** refuses to load a dylib
whose Team ID differs from the app's. Homebrew's dylib is ad-hoc signed (no matching Team ID),
so dyld aborts at launch with:

```
Library not loaded: /opt/homebrew/opt/openssl@3/lib/libssl.3.dylib
Reason: ... mapping process and mapped file have different Team IDs
```

surfaced as the generic **"La Vigie cannot be opened because of a problem"** dialog. This first
bit after PR #171 turned on real signing + hardened runtime; the OpenSSL dependency itself came
in with PR #172 (passkeys). Linking Homebrew's OpenSSL would also make the `.app` non-portable
(it wouldn't run on a machine lacking that dylib at that exact path).

**Fix (in `src-tauri/Cargo.toml`):** a macOS-scoped `openssl = { features = ["vendored"] }`
dependency statically links a bundled OpenSSL into our own signed binary. The `.app` is then
self-contained, portable, and library-validation clean. Scoped to macOS so Linux/CI keep using
system OpenSSL (no source build). Verify with `otool -L .../Contents/MacOS/vigie` — it should
show **no** `/opt/homebrew/...ssl` entries.

---

## Stable TCC grants across rebuilds (signed builds)

**Why:** macOS ties privacy grants (Screen Recording, Accessibility, …) to an app's
**code identity**. An *ad-hoc*-signed build gets a new `cdhash` on every rebuild, so macOS
silently revokes the grant each time you rebuild La Vigie. That makes agent GUI verification
(TASK-142 track (a), which needs Screen Recording) unusable during active development.

**Fix:** sign every build with a **stable** identity. macOS then keys the grant on the
identity + bundle id (a *designated requirement*), not the volatile `cdhash`, so the grant
**persists across rebuilds**. Both build paths are wired to the same identity so **one grant
covers both** the built `.app` and the `tauri dev` binary:

| Path | Where it's signed |
|------|-------------------|
| `tauri build` (the `.app`) | `src-tauri/tauri.conf.json` → `bundle.macOS.signingIdentity` |
| `tauri dev` (bare `target/debug/vigie`) | `src-tauri/.cargo/config.toml` `runner` → `src-tauri/scripts/dev-codesign-runner.sh` re-signs each `cargo run` |

Both produce the identical designated requirement (`identifier "com.lavigie" and anchor
apple generic and certificate leaf[CN] = "Apple Development: …"`), so one TCC grant covers both.

## Identity

Both paths use the generic selector **`Apple Development`**, which resolves to *your* local
Apple Development cert without committing anyone's personal identity to the repo. If you have
more than one matching cert, disambiguate with a more specific string (or a SHA-1) via
`LAVIGIE_DEV_SIGNING_IDENTITY` (dev binary) or `bundle.macOS.signingIdentity` / the
`APPLE_SIGNING_IDENTITY` env var (`tauri build`).

## Safety for CI / contributors without the cert

`dev-codesign-runner.sh` **no-ops** when no matching identity is in the keychain and **never
aborts** the run, so `cargo run` / `cargo test` are unaffected on CI or other machines (they run
ad-hoc, as before). CI does not run `tauri build`, so the `signingIdentity` config never affects
it. A contributor without an Apple Development cert should build with **`npm run tauri:build:unsigned`**
(ad-hoc) — the default `npm run tauri:build` hardcodes the `Apple Development` selector and will
fail to sign without a matching cert.

## Granting Screen Recording (one time)

After installing a signed build to `/Applications`: System Settings → Privacy & Security →
**Screen & System Audio Recording** → enable **La Vigie** → **⌘Q and reopen**. Because every
future build carries the same identity, the grant persists across rebuilds.

**Caveat:** the Apple Development cert renews ~Apr 2027. If the renewed cert's CN differs, the
designated requirement changes and you re-grant **once**. A self-signed cert with a 10-year
validity would avoid even that.
