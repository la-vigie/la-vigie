//! Tailnet remote-control server. Default-off; see
//! docs/superpowers/specs/2026-06-28-task-86-remote-control-poc-design.md.
pub mod auth;
pub mod commands;
pub mod server;
pub mod session;
pub mod tailscale;
pub mod webauthn;

use std::collections::HashMap;
use std::sync::Mutex;

use self::session::SessionStore;
use self::webauthn::{AuthState, RegState};

/// Live state of the remote server while enabled. `None` ⇒ feature is off and
/// every request is rejected.
#[derive(Default)]
pub struct RemoteState {
    pub active: Option<ActiveRemote>,
}

/// The running remote server's secret + reachability info and its shutdown
/// handle. Dropping/taking `active` and sending on `shutdown` tears it down.
pub struct ActiveRemote {
    pub token: String,
    pub magic_dns: String,
    pub port: u16,
    pub shutdown: tokio::sync::oneshot::Sender<()>,
    /// Held power assertion (macOS IOPMAssertion `PreventUserIdleSystemSleep`) that
    /// keeps the host reachable over the tailnet while remote is enabled.
    /// Released deterministically when this `ActiveRemote` is dropped/taken; the OS
    /// also reclaims it on process exit/crash. Best-effort: `None` when the
    /// assertion could not be acquired (remote still works, sleep just isn't held).
    pub keep_awake: Option<keepawake::KeepAwake>,
    /// Durable passkey sessions minted by a successful assertion. Cleared
    /// when remote is disabled or La Vigie restarts (this whole struct is dropped).
    pub sessions: SessionStore,
    /// In-flight WebAuthn ceremony state, keyed by an opaque ceremony id handed to
    /// the browser in the begin response and echoed back on finish. Removed on
    /// finish; abandoned entries die with `ActiveRemote`.
    pub reg_states: HashMap<String, RegState>,
    pub auth_states: HashMap<String, AuthState>,
}

/// Frontend-facing remote status (camelCase over IPC). `token`/`url` are only
/// present while active, so the desktop UI can show them.
#[derive(serde::Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RemoteStatus {
    pub active: bool,
    pub token: Option<String>,
    pub url: Option<String>,
    /// Whether a system-sleep-preventing power assertion is currently held.
    /// True only when remote is active *and* the assertion was
    /// acquired; false otherwise, so the UI can warn on best-effort failure.
    pub sleep_inhibited: bool,
}

/// Project `RemoteState` to the frontend-facing status.
pub fn status_of(state: &RemoteState) -> RemoteStatus {
    match &state.active {
        Some(a) => RemoteStatus {
            active: true,
            token: Some(a.token.clone()),
            url: Some(format!("https://{}/", a.magic_dns)),
            sleep_inhibited: a.keep_awake.is_some(),
        },
        None => RemoteStatus { active: false, token: None, url: None, sleep_inhibited: false },
    }
}

/// The `AppState` field type for the remote server.
pub type RemoteSlot = Mutex<RemoteState>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_of_reports_inactive_by_default() {
        let s = status_of(&RemoteState::default());
        assert!(!s.active);
        assert!(s.token.is_none());
        assert!(s.url.is_none());
        assert!(!s.sleep_inhibited);
    }

    #[test]
    fn status_of_reports_sleep_not_inhibited_when_assertion_absent() {
        // An active remote whose power assertion could not be acquired
        // (best-effort failure) reports active but sleep_inhibited == false.
        let (tx, _rx) = tokio::sync::oneshot::channel::<()>();
        let state = RemoteState {
            active: Some(ActiveRemote {
                token: "tok".into(),
                magic_dns: "host.example.ts.net".into(),
                port: 1234,
                shutdown: tx,
                keep_awake: None,
                sessions: Default::default(),
                reg_states: Default::default(),
                auth_states: Default::default(),
            }),
        };
        let s = status_of(&state);
        assert!(s.active);
        assert_eq!(s.url.as_deref(), Some("https://host.example.ts.net/"));
        assert!(!s.sleep_inhibited);
    }
}
