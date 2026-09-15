//! Live smoke test for the Stage 3 ACP connection driver (not run in CI — an
//! `.rs` file under `examples/` is never picked up by `cargo test`, and it
//! costs a small amount of real Claude usage).
//!
//! Spawns the real `@agentclientprotocol/claude-agent-acp` binary via `npx`,
//! then drives the session through `vigie_lib::acp::drive_connection` — the
//! exact core the Tauri driver (`acp::spawn_acp_session`) uses, not a second
//! hand-rolled client — sends a trivial prompt, and asserts a message chunk
//! containing "pong" plus a `TurnEnded` event both arrive.
//!
//! Run once manually:
//!
//!     cargo run --example acp_smoke
//!
//! Requires network access and a working Claude Code login (the ACP agent
//! shells out to the same auth as the `claude` CLI).

use std::process::Stdio;
use std::sync::{Arc, Mutex};

use agent_client_protocol::{Agent, ByteStreams, Client, ConnectionTo};
use futures::channel::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use vigie_lib::acp::{drive_connection, translate::AcpPhase, AcpEvent, DriverCommand};

#[tokio::main]
async fn main() {
    let cwd = std::env::temp_dir();

    let mut child = tokio::process::Command::new("npx")
        .args(["-y", "@agentclientprotocol/claude-agent-acp"])
        .current_dir(&cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn npx -y @agentclientprotocol/claude-agent-acp");

    let stdin = child.stdin.take().expect("piped stdin");
    let stdout = child.stdout.take().expect("piped stdout");
    let transport = ByteStreams::new(stdin.compat_write(), stdout.compat());

    let (cmd_tx, cmd_rx) = mpsc::unbounded::<DriverCommand>();
    let events: Arc<Mutex<Vec<AcpEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let events_for_cb = Arc::clone(&events);

    // When TurnEnded arrives, drop the command sender so `cmd_rx` closes and
    // `drive_connection`'s select! loop exits via its own `None => break` —
    // the real production shutdown path, not a test-only escape hatch.
    let cmd_tx_slot: Mutex<Option<mpsc::UnboundedSender<DriverCommand>>> = Mutex::new(Some(cmd_tx));
    let on_event = move |ev: AcpEvent| {
        let is_turn_ended = matches!(ev, AcpEvent::TurnEnded { .. });
        eprintln!("[acp-smoke] event: {ev:?}");
        events_for_cb.lock().expect("events mutex poisoned").push(ev);
        if is_turn_ended {
            let _ = cmd_tx_slot.lock().expect("cmd_tx_slot mutex poisoned").take();
        }
    };
    let on_status = |phase: AcpPhase| eprintln!("[acp-smoke] status: {phase:?}");

    let result = Client
        .builder()
        .name("acp-smoke")
        .connect_with(transport, move |cx: ConnectionTo<Agent>| async move {
            drive_connection(
                cx,
                cwd,
                Some("Reply with the single word: pong".to_string()),
                None,
                0,
                None,
                None, // resume: fresh session for the smoke test
                cmd_rx,
                &on_event,
                &on_event, // on_replay: unused on a fresh session
                &on_status,
            )
            .await
        })
        .await;

    let _ = child.start_kill();

    result.expect("ACP connection failed");

    let collected = events.lock().expect("events mutex poisoned");
    let assembled_text: String = collected
        .iter()
        .filter_map(|ev| match ev {
            AcpEvent::MessageChunk { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    let saw_turn_ended = collected.iter().any(|ev| matches!(ev, AcpEvent::TurnEnded { .. }));

    eprintln!("[acp-smoke] assembled message text: {assembled_text:?}");
    assert!(
        assembled_text.to_lowercase().contains("pong"),
        "expected a message chunk containing 'pong', got: {assembled_text:?}"
    );
    assert!(saw_turn_ended, "expected a TurnEnded event, got: {collected:?}");

    println!("acp_smoke: PASS (saw 'pong' + TurnEnded)");
}
