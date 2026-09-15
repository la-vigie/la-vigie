//! Durable-session plumbing for passkey remote auth. A successful
//! passkey assertion mints a random session id, stored HERE in-memory (in
//! `ActiveRemote`) and handed to the browser as an HttpOnly cookie. Subsequent
//! requests present the cookie instead of the fragile sessionStorage bearer,
//! surviving iOS tab backgrounding/lock/close.
//!
//! Everything in this file is pure (no I/O, no wall clock) — the caller passes
//! `now_ms` explicitly — so the store and cookie helpers are unit-tested below.
//! `now_ms()` (the one impure helper) reads the system clock for production use.

use std::collections::HashMap;

/// Cookie name carrying the durable session id.
pub const SESSION_COOKIE: &str = "vigie_session";

/// Idle lifetime of a session, refreshed (rolled) on every authenticated
/// request. Sessions also die when La Vigie restarts (they live in `ActiveRemote`,
/// which is in-memory) — after which the phone re-presents its passkey (one tap,
/// no QR). 30 days comfortably covers day-to-day phone use.
pub const SESSION_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// Cookie `Max-Age` (seconds) — matches the server idle TTL so the browser keeps
/// sending the cookie for as long as the server would honor it.
pub const SESSION_COOKIE_MAX_AGE_SECS: u64 = SESSION_TTL_MS / 1000;

/// Wall-clock milliseconds since the Unix epoch. The ONLY impure function here;
/// tests inject `now_ms` directly and never call this.
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Extract the `vigie_session` value from a `Cookie` request header. Returns the
/// first non-empty match. Handles the standard `a=b; c=d` list with surrounding
/// whitespace; ignores other cookies.
pub fn parse_session_cookie(cookie_header: Option<&str>) -> Option<String> {
    let raw = cookie_header?;
    let prefix = format!("{SESSION_COOKIE}=");
    for part in raw.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix(&prefix) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// Build the `Set-Cookie` value that installs a session. HttpOnly (JS can't read
/// it → not exfiltratable via XSS), Secure (served over `tailscale serve` TLS),
/// SameSite=Strict (no cross-site sends), Path=/ (whole remote surface).
pub fn build_session_cookie(sid: &str, max_age_secs: u64) -> String {
    format!(
        "{SESSION_COOKIE}={sid}; Max-Age={max_age_secs}; Path=/; HttpOnly; Secure; SameSite=Strict"
    )
}

/// Build the `Set-Cookie` value that clears the session (logout). `Max-Age=0`
/// tells the browser to drop it immediately.
pub fn build_clearing_cookie() -> String {
    format!("{SESSION_COOKIE}=; Max-Age=0; Path=/; HttpOnly; Secure; SameSite=Strict")
}

#[derive(Debug, Clone)]
struct Session {
    /// The passkey credential id (base64url) that minted this session. Revoking
    /// that credential drops every session it minted (`remove_by_credential`).
    credential_id: String,
    /// Absolute expiry in epoch-ms; rolled forward on each authenticated request.
    expiry_ms: u64,
}

/// In-memory session table. Lives in `ActiveRemote`, so it is cleared whenever
/// remote is disabled or La Vigie restarts.
#[derive(Default)]
pub struct SessionStore {
    map: HashMap<String, Session>,
}

impl SessionStore {
    /// Register a freshly minted session id bound to the authenticating credential.
    pub fn insert(&mut self, sid: String, credential_id: String, now_ms: u64, ttl_ms: u64) {
        self.map.insert(
            sid,
            Session { credential_id, expiry_ms: now_ms.saturating_add(ttl_ms) },
        );
    }

    /// Validate a presented session id. On success returns the bound credential id
    /// and rolls the expiry forward (sliding idle window). An expired session is
    /// pruned and treated as absent.
    pub fn validate_and_roll(&mut self, sid: &str, now_ms: u64, ttl_ms: u64) -> Option<String> {
        match self.map.get_mut(sid) {
            Some(s) if s.expiry_ms > now_ms => {
                s.expiry_ms = now_ms.saturating_add(ttl_ms);
                Some(s.credential_id.clone())
            }
            Some(_) => {
                self.map.remove(sid);
                None
            }
            None => None,
        }
    }

    /// Drop a single session (logout). Returns whether it existed.
    pub fn remove(&mut self, sid: &str) -> bool {
        self.map.remove(sid).is_some()
    }

    /// Drop every session minted by a given credential (credential revocation).
    /// Returns how many were removed.
    pub fn remove_by_credential(&mut self, credential_id: &str) -> usize {
        let before = self.map.len();
        self.map.retain(|_, s| s.credential_id != credential_id);
        before - self.map.len()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: u64 = 1000;

    #[test]
    fn parse_session_cookie_extracts_value() {
        assert_eq!(parse_session_cookie(Some("vigie_session=abc")).as_deref(), Some("abc"));
        assert_eq!(
            parse_session_cookie(Some("foo=1; vigie_session=abc; bar=2")).as_deref(),
            Some("abc")
        );
        assert_eq!(
            parse_session_cookie(Some("  vigie_session=abc  ")).as_deref(),
            Some("abc")
        );
        assert_eq!(parse_session_cookie(Some("other=1; foo=2")), None);
        assert_eq!(parse_session_cookie(Some("vigie_session=")), None);
        assert_eq!(parse_session_cookie(Some("vigie_session=  ")), None);
        assert_eq!(parse_session_cookie(None), None);
    }

    #[test]
    fn parse_does_not_match_a_cookie_that_only_ends_with_the_name() {
        // `xvigie_session=abc` must NOT match `vigie_session`.
        assert_eq!(parse_session_cookie(Some("xvigie_session=abc")), None);
    }

    #[test]
    fn build_session_cookie_has_hardened_attributes() {
        let c = build_session_cookie("sid123", 60);
        assert!(c.starts_with("vigie_session=sid123;"));
        assert!(c.contains("Max-Age=60"));
        assert!(c.contains("Path=/"));
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("Secure"));
        assert!(c.contains("SameSite=Strict"));
    }

    #[test]
    fn clearing_cookie_expires_immediately() {
        let c = build_clearing_cookie();
        assert!(c.contains("Max-Age=0"));
        assert!(c.contains("HttpOnly"));
    }

    #[test]
    fn validate_returns_credential_and_rolls_expiry() {
        let mut s = SessionStore::default();
        s.insert("sid".into(), "cred".into(), 0, TTL);
        // Valid before expiry; rolls the window forward.
        assert_eq!(s.validate_and_roll("sid", 500, TTL).as_deref(), Some("cred"));
        // Because it rolled to 500+1000=1500, it is still valid at 1400.
        assert_eq!(s.validate_and_roll("sid", 1400, TTL).as_deref(), Some("cred"));
    }

    #[test]
    fn expired_session_is_pruned_and_rejected() {
        let mut s = SessionStore::default();
        s.insert("sid".into(), "cred".into(), 0, TTL);
        // At exactly ttl the session is expired (strictly-greater check).
        assert_eq!(s.validate_and_roll("sid", 1000, TTL), None);
        assert!(s.is_empty());
        // A second attempt sees nothing.
        assert_eq!(s.validate_and_roll("sid", 1001, TTL), None);
    }

    #[test]
    fn unknown_session_is_rejected() {
        let mut s = SessionStore::default();
        assert_eq!(s.validate_and_roll("nope", 0, TTL), None);
    }

    #[test]
    fn remove_drops_single_session() {
        let mut s = SessionStore::default();
        s.insert("a".into(), "cred".into(), 0, TTL);
        assert!(s.remove("a"));
        assert!(!s.remove("a"));
        assert_eq!(s.validate_and_roll("a", 0, TTL), None);
    }

    #[test]
    fn remove_by_credential_drops_all_sessions_of_that_credential() {
        let mut s = SessionStore::default();
        s.insert("a".into(), "phone".into(), 0, TTL);
        s.insert("b".into(), "phone".into(), 0, TTL);
        s.insert("c".into(), "laptop".into(), 0, TTL);
        assert_eq!(s.remove_by_credential("phone"), 2);
        assert_eq!(s.len(), 1);
        assert_eq!(s.validate_and_roll("c", 0, TTL).as_deref(), Some("laptop"));
        // Revoking again removes nothing.
        assert_eq!(s.remove_by_credential("phone"), 0);
    }
}
