//! Hydrate the current process environment from the user's login shell so a
//! GUI-launched `.app` (minimal LaunchServices environment) behaves like the
//! terminal-launched `tauri dev` build.
//!
//! `parse_env` is pure and unit-tested. `capture_login_env` (spawns the login
//! shell) and `hydrate` (mutates the global process env) touch live system
//! state and are covered by manual integration testing in the built `.app`.

use std::process::Command;

/// Parse the NUL-delimited output of `env -0` into `(key, value)` pairs.
///
/// Each record is `KEY=VALUE`. The key must match `[A-Za-z_][A-Za-z0-9_]*`;
/// records with a malformed or empty key (or no `=`) are dropped. The value is
/// kept verbatim (it may contain `=` or newlines). Empty records (e.g. the
/// trailing one after the final NUL) are ignored.
pub fn parse_env(output: &str) -> Vec<(String, String)> {
    output
        .split('\0')
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let (key, value) = record.split_once('=')?;
            if is_valid_key(key) {
                Some((key.to_string(), value.to_string()))
            } else {
                None
            }
        })
        .collect()
}

/// True if `key` is a valid env var name: `[A-Za-z_][A-Za-z0-9_]*`.
fn is_valid_key(key: &str) -> bool {
    let mut chars = key.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Capture the user's login-shell environment by running `$SHELL -ilc 'env -0'`.
/// Returns the parsed pairs, or an empty vec on any failure (spawn error or
/// non-zero exit; non-UTF-8 bytes are lossily decoded, not a failure). `$SHELL` falls back to `/bin/zsh`.
fn capture_login_env() -> Vec<(String, String)> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let output = Command::new(&shell).args(["-ilc", "env -0"]).output();

    match output {
        Ok(out) if out.status.success() => parse_env(&String::from_utf8_lossy(&out.stdout)),
        _ => vec![],
    }
}

/// Repair the current process environment for a GUI-launched `.app`.
///
/// Overlays the login-shell environment (login-shell values win, restoring the
/// real `PATH`), then sets `TERM`/`COLORTERM` defaults only if absent — so a
/// real terminal's values in `tauri dev` are preserved and the bundle gets sane
/// defaults matching the xterm.js frontend.
///
/// MUST be called before any threads are spawned: `std::env::set_var` is not
/// thread-safe.
pub fn hydrate() {
    for (key, value) in capture_login_env() {
        std::env::set_var(key, value);
    }
    if std::env::var_os("TERM").is_none() {
        std::env::set_var("TERM", "xterm-256color");
    }
    if std::env::var_os("COLORTERM").is_none() {
        std::env::set_var("COLORTERM", "truecolor");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_pairs() {
        let out = "PATH=/usr/bin\0HOME=/home/user\0";
        assert_eq!(
            parse_env(out),
            vec![
                ("PATH".to_string(), "/usr/bin".to_string()),
                ("HOME".to_string(), "/home/user".to_string()),
            ]
        );
    }

    #[test]
    fn keeps_value_containing_equals() {
        let out = "FOO=a=b=c\0";
        assert_eq!(parse_env(out), vec![("FOO".to_string(), "a=b=c".to_string())]);
    }

    #[test]
    fn keeps_value_containing_newline() {
        let out = "MULTI=line1\nline2\0NEXT=ok\0";
        assert_eq!(
            parse_env(out),
            vec![
                ("MULTI".to_string(), "line1\nline2".to_string()),
                ("NEXT".to_string(), "ok".to_string()),
            ]
        );
    }

    #[test]
    fn drops_malformed_keys() {
        // leading digit, key with a space, empty key, and a record with no '=' are all dropped
        let out = "1BAD=x\0BAD KEY=y\0NOEQUALS\0=value\0GOOD=z\0";
        assert_eq!(parse_env(out), vec![("GOOD".to_string(), "z".to_string())]);
    }

    #[test]
    fn empty_input_yields_empty() {
        assert_eq!(parse_env(""), Vec::<(String, String)>::new());
    }
}
