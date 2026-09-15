//! WebAuthn passkey ceremonies for durable remote auth. Thin wrappers
//! over `webauthn-rs` that bind the Relying Party to the tailnet MagicDNS host —
//! RP ID = the hostname, origin = `https://<host>` (served over `tailscale serve`
//! TLS, so the browser sees a secure context, WebAuthn's hard requirement).
//!
//! No I/O and no wall clock: a fresh `Webauthn` is built per call from the passed
//! `magic_dns`. Ceremony state (`PasskeyRegistration`/`PasskeyAuthentication`) is
//! returned to the caller, which holds it in-memory in `ActiveRemote` between the
//! begin/finish halves. Errors are flattened to `String` for the axum handlers.

use webauthn_rs::prelude::*;

/// The single remote "user". The passkey RP has exactly one account — whoever
/// controls this Mac — so a fixed, stable handle is correct (it MUST NOT change
/// between registration and later authentications). Not secret.
pub fn remote_user_id() -> Uuid {
    // Stable arbitrary namespace UUID for La Vigie remote.
    Uuid::from_u128(0xac20240_1111_4222_8333_444455556666)
}

const USER_NAME: &str = "la-vigie-remote";
const USER_DISPLAY_NAME: &str = "La Vigie Remote";

/// In-progress ceremony state, held in-memory by the caller between begin/finish.
pub type RegState = PasskeyRegistration;
pub type AuthState = PasskeyAuthentication;

/// Build a `Webauthn` bound to the MagicDNS host. RP ID must be a registrable
/// suffix of the origin — here they share the exact host, so this holds for any
/// `*.ts.net` MagicDNS name.
fn build(magic_dns: &str) -> Result<Webauthn, String> {
    if magic_dns.is_empty() {
        return Err("no MagicDNS host — remote not active".to_string());
    }
    let origin = Url::parse(&format!("https://{magic_dns}"))
        .map_err(|e| format!("invalid RP origin for {magic_dns}: {e}"))?;
    let builder = WebauthnBuilder::new(magic_dns, &origin)
        .map_err(|e| format!("webauthn builder ({magic_dns}): {e}"))?;
    builder
        .rp_name("La Vigie Remote")
        .build()
        .map_err(|e| format!("webauthn build ({magic_dns}): {e}"))
}

/// Begin registering a new passkey. `exclude` is the set of already-registered
/// credential ids, so a device won't silently double-register.
pub fn start_registration(
    magic_dns: &str,
    exclude: Vec<CredentialID>,
) -> Result<(CreationChallengeResponse, RegState), String> {
    let wan = build(magic_dns)?;
    let exclude = if exclude.is_empty() { None } else { Some(exclude) };
    wan.start_passkey_registration(remote_user_id(), USER_NAME, USER_DISPLAY_NAME, exclude)
        .map_err(|e| format!("start registration: {e}"))
}

/// Finish registration, yielding the durable `Passkey` to persist.
pub fn finish_registration(
    magic_dns: &str,
    cred: &RegisterPublicKeyCredential,
    state: &RegState,
) -> Result<Passkey, String> {
    let wan = build(magic_dns)?;
    wan.finish_passkey_registration(cred, state)
        .map_err(|e| format!("finish registration: {e}"))
}

/// Begin authentication against the caller's known passkeys.
pub fn start_authentication(
    magic_dns: &str,
    passkeys: &[Passkey],
) -> Result<(RequestChallengeResponse, AuthState), String> {
    let wan = build(magic_dns)?;
    wan.start_passkey_authentication(passkeys)
        .map_err(|e| format!("start authentication: {e}"))
}

/// Finish authentication, yielding the result (which credential, counter, whether
/// the stored passkey needs a counter update).
pub fn finish_authentication(
    magic_dns: &str,
    cred: &PublicKeyCredential,
    state: &AuthState,
) -> Result<AuthenticationResult, String> {
    let wan = build(magic_dns)?;
    wan.finish_passkey_authentication(cred, state)
        .map_err(|e| format!("finish authentication: {e}"))
}

/// Base64url-encode a credential id for use as the SQLite primary key and for the
/// client-facing credential list.
pub fn cred_id_b64(id: &CredentialID) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(id.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_user_id_is_stable() {
        assert_eq!(remote_user_id(), remote_user_id());
    }

    #[test]
    fn build_succeeds_for_a_magic_dns_host() {
        // A normal tailnet MagicDNS name yields a working RP (RP ID == origin host).
        assert!(build("mac.tail-scale.ts.net").is_ok());
    }

    #[test]
    fn build_rejects_empty_host() {
        assert!(build("").is_err());
    }

    #[test]
    fn start_registration_produces_a_challenge() {
        let (ccr, _state) = start_registration("mac.tail-scale.ts.net", Vec::new())
            .expect("registration should start");
        // The challenge response serializes to a `{ publicKey: {...} }` object the
        // browser can consume via navigator.credentials.create.
        let json = serde_json::to_value(&ccr).unwrap();
        assert!(json.get("publicKey").is_some());
        assert!(json["publicKey"].get("challenge").is_some());
        assert_eq!(json["publicKey"]["rp"]["id"], "mac.tail-scale.ts.net");
    }

    #[test]
    fn start_authentication_with_no_passkeys_is_guarded_at_the_handler() {
        // webauthn-rs does NOT reject an empty allow-list here (it would produce a
        // challenge no authenticator can satisfy), so the "no passkeys registered"
        // guard lives in `authenticate_begin_handler` (returns 409). Document that
        // this layer is permissive.
        assert!(start_authentication("mac.tail-scale.ts.net", &[]).is_ok());
    }

    #[test]
    fn cred_id_b64_is_url_safe_no_pad() {
        let id: CredentialID = vec![0xff, 0xfe, 0xfd].into();
        let s = cred_id_b64(&id);
        assert!(!s.contains('+') && !s.contains('/') && !s.contains('='));
    }
}
