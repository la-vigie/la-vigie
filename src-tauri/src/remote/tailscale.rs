//! `tailscale` CLI wrapper for the remote server: resolve the binary, run
//! `serve`/`status` with argv vectors (no shell), and parse their JSON.
//!
//! The pure parsers (`magic_dns_from_status`, `funnel_active`) are unit-tested
//! against `fixtures/`. The async wrappers that shell out are glue — verified
//! live in `npm run tauri dev`, not unit-tested.

use std::path::PathBuf;

use serde_json::Value;

/// Extract the tailnet MagicDNS name from `tailscale status --json`
/// (`Self.DNSName`), stripping the trailing dot.
pub fn magic_dns_from_status(json: &str) -> Result<String, String> {
    let v: Value = serde_json::from_str(json).map_err(|e| format!("{e:#}"))?;
    let name = v
        .get("Self")
        .and_then(|s| s.get("DNSName"))
        .and_then(|d| d.as_str())
        .ok_or_else(|| "tailscale status: missing Self.DNSName".to_string())?;
    let name = name.trim_end_matches('.').to_string();
    if name.is_empty() {
        return Err("tailscale status: empty Self.DNSName".to_string());
    }
    Ok(name)
}

/// True if `tailscale serve status --json` shows ANY Funnel mapping enabled
/// (any `true` under `AllowFunnel`). Used to refuse enabling when Funnel —
/// public internet exposure — is configured.
pub fn funnel_active(serve_status_json: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(serve_status_json) else {
        // Unparseable serve status ⇒ treat as unsafe (assume funnel) and refuse.
        return true;
    };
    match v.get("AllowFunnel") {
        // Absent or explicit null ⇒ no funnel configured.
        None | Some(Value::Null) => false,
        Some(f) => match f.as_object() {
            Some(map) => map.values().any(|val| val.as_bool() == Some(true)),
            // Present and non-null but not an object ⇒ fail-safe: treat as funnel-active.
            None => true,
        },
    }
}

/// Resolve the `tailscale` binary: current PATH first, then the standalone
/// macOS app bundle, then Homebrew/local. Falls back to the bare name.
pub fn find_tailscale() -> PathBuf {
    let path_dirs = std::env::var("PATH")
        .map(|p| p.split(':').filter(|s| !s.is_empty()).map(PathBuf::from).collect::<Vec<_>>())
        .unwrap_or_default();
    let candidates = vec![
        PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale"),
        PathBuf::from("/opt/homebrew/bin/tailscale"),
        PathBuf::from("/usr/local/bin/tailscale"),
    ];
    crate::claude_path::resolve_in("tailscale", &path_dirs, &candidates)
        .unwrap_or_else(|| PathBuf::from("tailscale"))
}

/// Run `tailscale <args>` capturing stdout; error includes stderr on non-zero.
async fn run(args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new(find_tailscale())
        .args(args)
        .output()
        .await
        .map_err(|e| format!("tailscale {}: {e:#}", args.join(" ")))?;
    if !out.status.success() {
        return Err(format!(
            "tailscale {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// `tailscale status --json` → MagicDNS name.
pub async fn magic_dns_name() -> Result<String, String> {
    let json = run(&["status", "--json"]).await?;
    magic_dns_from_status(&json)
}

/// `tailscale serve --bg --https 443 http://127.0.0.1:<port>` — tailnet-only
/// reverse proxy with automatic TLS on the MagicDNS name.
pub async fn serve_start(port: u16) -> Result<(), String> {
    let target = format!("http://127.0.0.1:{port}");
    run(&["serve", "--bg", "--https", "443", &target]).await.map(|_| ())
}

/// `tailscale serve reset` — the kill switch.
pub async fn serve_reset() -> Result<(), String> {
    run(&["serve", "reset"]).await.map(|_| ())
}

/// `tailscale serve status --json` → whether Funnel is active.
pub async fn serve_funnel_active() -> Result<bool, String> {
    let json = run(&["serve", "status", "--json"]).await?;
    Ok(funnel_active(&json))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn magic_dns_from_status_strips_trailing_dot() {
        let json = include_str!("fixtures/status.json");
        assert_eq!(magic_dns_from_status(json).unwrap(), "mac-studio.tail-scale.ts.net");
    }

    #[test]
    fn magic_dns_from_status_errors_without_self() {
        assert!(magic_dns_from_status("{}").is_err());
    }

    #[test]
    fn funnel_active_true_when_allowfunnel_has_true() {
        let json = include_str!("fixtures/serve-status-funnel.json");
        assert!(funnel_active(json));
    }

    #[test]
    fn funnel_active_false_when_no_funnel() {
        let json = include_str!("fixtures/serve-status-no-funnel.json");
        assert!(!funnel_active(json));
    }

    #[test]
    fn funnel_active_true_on_unparseable_status() {
        // Fail safe: unparseable ⇒ assume funnel ⇒ refuse to enable.
        assert!(funnel_active("not json"));
    }

    #[test]
    fn funnel_active_true_when_allowfunnel_present_but_not_object() {
        // AllowFunnel is present and non-null but not an object (unexpected schema)
        // ⇒ fail-safe: treat as funnel-active and refuse to enable.
        assert!(funnel_active(r#"{"AllowFunnel": true}"#));
        assert!(funnel_active(r#"{"AllowFunnel": "yes"}"#));
    }

    #[test]
    fn funnel_active_false_when_allowfunnel_absent_or_null() {
        // Absent or explicit null ⇒ no funnel configured.
        assert!(!funnel_active(r#"{}"#));
        assert!(!funnel_active(r#"{"AllowFunnel": null}"#));
    }
}
