//! Pure auth helpers for the remote server: constant-time token compare,
//! bearer extraction, and the Host-header allowlist (anti DNS-rebinding).
//! No I/O — unit-tested below.

/// Constant-time byte-slice equality. Length is treated as public (the token
/// length is fixed and not secret), so a length mismatch short-circuits; equal
/// lengths are compared without data-dependent branches.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Extract the token from an `Authorization: Bearer <token>` header value.
/// Returns `None` if the header is missing or not a non-empty bearer token.
pub fn parse_bearer(header: Option<&str>) -> Option<&str> {
    let raw = header?.trim();
    let token = raw.strip_prefix("Bearer ")?.trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

/// True if the request `Host` header matches the tailnet MagicDNS name.
/// Strips an optional `:port`, compares case-insensitively, and tolerates a
/// trailing dot on either side (MagicDNS names are often fully-qualified).
pub fn host_allowed(host_header: Option<&str>, magic_dns: &str) -> bool {
    let Some(host) = host_header else {
        return false;
    };
    // Only the host portion is compared; IPv6 literals are not handled because
    // MagicDNS names are always hostnames, not raw IP addresses.
    let host = host.split(':').next().unwrap_or("").trim_end_matches('.');
    let want = magic_dns.trim_end_matches('.');
    !want.is_empty() && host.eq_ignore_ascii_case(want)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_equal_and_rejects_different() {
        assert!(constant_time_eq(b"abc123", b"abc123"));
        assert!(!constant_time_eq(b"abc123", b"abc124"));
        assert!(!constant_time_eq(b"abc", b"abc123")); // length mismatch
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn parse_bearer_extracts_token() {
        assert_eq!(parse_bearer(Some("Bearer xyz")), Some("xyz"));
        assert_eq!(parse_bearer(Some("Bearer   xyz  ")), Some("xyz"));
        assert_eq!(parse_bearer(Some("xyz")), None);
        assert_eq!(parse_bearer(Some("Bearer ")), None);
        assert_eq!(parse_bearer(None), None);
    }

    #[test]
    fn host_allowed_matches_magic_dns() {
        assert!(host_allowed(Some("mac.tail-scale.ts.net"), "mac.tail-scale.ts.net"));
        assert!(host_allowed(Some("mac.tail-scale.ts.net:443"), "mac.tail-scale.ts.net"));
        assert!(host_allowed(Some("MAC.tail-scale.ts.net"), "mac.tail-scale.ts.net."));
        assert!(!host_allowed(Some("evil.example.com"), "mac.tail-scale.ts.net"));
        assert!(!host_allowed(None, "mac.tail-scale.ts.net"));
        assert!(!host_allowed(Some("anything"), ""));
    }
}
