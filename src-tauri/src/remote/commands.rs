//! Tauri commands driving the remote server lifecycle. Pure pieces live in
//! `auth`/`tailscale`/`server`; this is the orchestration glue (verify live).

use tauri::State;
use uuid::Uuid;

use crate::remote::{server, status_of, tailscale, ActiveRemote, RemoteStatus};
use crate::state::AppState;

/// Mint a high-entropy pairing token (two v4 UUIDs ⇒ 64 hex chars). Keychain
/// storage + hashing are deferred (AC2-57); the token is in-memory only.
fn mint_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

/// Acquire a system-sleep-preventing power assertion for the lifetime of the
/// enabled remote server (AC2-104). On macOS this is an IOPMAssertion
/// (`PreventUserIdleSystemSleep`) released deterministically when the returned
/// handle is dropped. `idle` only — display sleep is left untouched.
///
/// Best-effort: failures are logged and return `None` (remote control still
/// works; the host just isn't kept awake). Note: the assertion is honored only
/// on AC power — on battery the Mac may idle-sleep regardless.
fn acquire_keep_awake() -> Option<keepawake::KeepAwake> {
    match keepawake::Builder::default()
        .idle(true)
        .reason("Remote control enabled")
        .app_name("La Vigie")
        .app_reverse_domain("com.lavigie")
        .create()
    {
        Ok(k) => Some(k),
        Err(e) => {
            eprintln!("AC2-104: failed to acquire sleep-prevention assertion: {e:#}");
            None
        }
    }
}

/// Enable remote control: mint a token, start the loopback server, front it with
/// `tailscale serve`, assert no Funnel, resolve the MagicDNS name, and store the
/// active state. Rolls back the server + serve on any failure.
#[tauri::command]
pub async fn enable_remote(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
) -> Result<RemoteStatus, String> {
    // Idempotent: if already active, just report status (no await under lock).
    {
        let remote = state.remote.lock().map_err(|e| format!("{e:#}"))?;
        if remote.active.is_some() {
            return Ok(status_of(&remote));
        }
    }

    let magic_dns = tailscale::magic_dns_name().await?;
    let token = mint_token();

    let (port, shutdown) = server::start_remote_server(app.clone())
        .await
        .map_err(|e| format!("failed to start remote server: {e:#}"))?;

    // Front with serve; on failure tear down both serve mapping and listener.
    if let Err(e) = tailscale::serve_start(port).await {
        let _ = tailscale::serve_reset().await;
        let _ = shutdown.send(());
        return Err(e);
    }

    // Refuse if Funnel (public exposure) is configured; roll back fully.
    match tailscale::serve_funnel_active().await {
        Ok(true) => {
            let _ = tailscale::serve_reset().await;
            let _ = shutdown.send(());
            return Err("Tailscale Funnel is enabled — refusing to expose La Vigie publicly. Disable Funnel and retry.".to_string());
        }
        Ok(false) => {}
        Err(e) => {
            let _ = tailscale::serve_reset().await;
            let _ = shutdown.send(());
            return Err(e);
        }
    }

    // Re-check under the final lock; a concurrent enable may have won the race.
    // Extract a plain bool so the guard is dropped before any await point.
    let already_active = {
        let remote = state.remote.lock().map_err(|e| format!("{e:#}"))?;
        remote.active.is_some()
    }; // guard dropped here — no MutexGuard is live across any await below
    if already_active {
        // Lost the race: tear down our just-created server; keep the winner's.
        let _ = tailscale::serve_reset().await;
        let _ = shutdown.send(());
        let remote = state.remote.lock().map_err(|e| format!("{e:#}"))?;
        return Ok(status_of(&remote));
    }
    // Hold a system-sleep assertion so the tailnet host stays reachable while
    // remote is on (AC2-104). Best-effort: acquire failure leaves it `None` and
    // does not block enable. Acquired before the lock — `keepawake` is synchronous.
    let keep_awake = acquire_keep_awake();
    let mut remote = state.remote.lock().map_err(|e| format!("{e:#}"))?;
    remote.active = Some(ActiveRemote { token, magic_dns, port, shutdown, keep_awake });
    Ok(status_of(&remote))
}

/// Kill switch: tear down the listener and reset `tailscale serve`. Dropping the
/// active state invalidates the token (subsequent requests 401).
#[tauri::command]
pub async fn disable_remote(state: State<'_, AppState>) -> Result<RemoteStatus, String> {
    let active = {
        let mut remote = state.remote.lock().map_err(|e| format!("{e:#}"))?;
        remote.active.take()
    }; // guard dropped here — no MutexGuard is live across any await below
    if let Some(a) = active {
        let _ = a.shutdown.send(());
        // Dropping `a` releases the held power assertion (AC2-104).
        drop(a.keep_awake);
        let _ = tailscale::serve_reset().await;
    }
    Ok(RemoteStatus { active: false, token: None, url: None, sleep_inhibited: false })
}

/// Current remote status (active flag + token/url when active).
#[tauri::command]
pub async fn remote_status(state: State<'_, AppState>) -> Result<RemoteStatus, String> {
    let remote = state.remote.lock().map_err(|e| format!("{e:#}"))?;
    Ok(status_of(&remote))
}
