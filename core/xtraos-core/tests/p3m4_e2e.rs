//! P3-M4 sandbox channel (docs/08 P3-SB-*, ADR 0040), without Windows Sandbox: the inner
//! agent runs in --test-on-host mode (read-only tools only) on a temp exchange folder.
//! The live Sandbox test (start → work inside → stop) is human (after enabling the feature).
mod common;

use std::time::{Duration, Instant};

use common::{code, Core};
use serde_json::json;

fn exe() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_xtraos-core"))
}

// P3-SB-01..05
#[tokio::test(flavor = "multi_thread")]
async fn sb01_05_channel_tokens_test_mode_and_stop() {
    let root = std::env::temp_dir().join(format!("xtraos-sb-{}", common::rand_hex(3)));
    let (ex, sess) = (root.join("exchange"), root.join("session"));
    std::fs::create_dir_all(&sess).unwrap();
    std::fs::write(sess.join("session.token"), "tok-1").unwrap();
    std::env::set_var("XTRAOS_SANDBOX_EXCHANGE", &ex);
    std::env::set_var("XTRAOS_SANDBOX_SESSION", &sess);

    // Outside a Sandbox, without test mode, the inner agent refuses to run.
    let refused = std::process::Command::new(exe())
        .args([
            "--sandbox-agent",
            &ex.to_string_lossy(),
            "--session",
            &sess.to_string_lossy(),
        ])
        .env("USERNAME", "someone")
        .status()
        .unwrap();
    assert_eq!(refused.code(), Some(3), "SB-01: refuses outside a Sandbox");

    let mut agent = std::process::Command::new(exe())
        .args([
            "--sandbox-agent",
            &ex.to_string_lossy(),
            "--session",
            &sess.to_string_lossy(),
            "--test-on-host",
        ])
        .spawn()
        .unwrap();
    let t0 = Instant::now();
    while !ex.join("ready.json").exists() {
        assert!(t0.elapsed() < Duration::from_secs(10), "no heartbeat");
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut core = Core::start().await;
    let st = core.ok("sandbox", json!({"op": "status"})).await;
    eprintln!("SB status: {}", st["text"]);
    assert_eq!(st["running"], true);

    let lw = core
        .ok("sandbox", json!({"op": "call", "tool": "list_windows"}))
        .await;
    assert_eq!(lw["via"], "sandbox");
    assert!(lw["windows"].is_array(), "{lw}");
    eprintln!(
        "SB-02 ok: list_windows through the channel ({} windows)",
        lw["windows"].as_array().unwrap().len()
    );

    let click = core
        .call(
            "sandbox",
            json!({"op": "call", "tool": "click", "args": {"x": 1, "y": 1}}),
        )
        .await;
    assert_eq!(
        code(&click),
        "BLOCKED_BY_POLICY",
        "SB-03: input refused in test mode: {click}"
    );
    let nested = core
        .call("sandbox", json!({"op": "call", "tool": "sandbox"}))
        .await;
    assert_eq!(code(&nested), "INVALID_ARGS");

    std::fs::write(sess.join("session.token"), "tok-2").unwrap();
    let wrong = core
        .call("sandbox", json!({"op": "call", "tool": "status"}))
        .await;
    assert_eq!(
        code(&wrong),
        "BLOCKED_BY_POLICY",
        "SB-04: wrong token: {wrong}"
    );
    std::fs::write(sess.join("session.token"), "tok-1").unwrap();

    let stop = core.ok("sandbox", json!({"op": "stop"})).await;
    eprintln!("SB-05 stop: {}", stop["text"]);
    let st = agent.wait().unwrap();
    assert_eq!(
        st.code(),
        Some(0),
        "test agent ended (the PC is never shut down in test mode)"
    );
    let gone = core
        .call("sandbox", json!({"op": "call", "tool": "status"}))
        .await;
    assert_eq!(code(&gone), "NEEDS_HUMAN", "no sandbox running: {gone}");
    let start = core.call("sandbox", json!({"op": "start"})).await;
    if !std::path::Path::new(r"C:\Windows\System32\WindowsSandbox.exe").exists() {
        assert_eq!(
            code(&start),
            "NEEDS_HUMAN",
            "not installed → the user enables it: {start}"
        );
        eprintln!("SB start without the feature: {}", start["error"]["hint"]);
    }
    core.stop().await;
    std::env::remove_var("XTRAOS_SANDBOX_EXCHANGE");
    std::env::remove_var("XTRAOS_SANDBOX_SESSION");
    let _ = std::fs::remove_dir_all(root);
}

// P3-SB-LIVE (human, after enabling Windows Sandbox; XTRAOS_TEST_SANDBOX=1): start a real
// Sandbox, open Notepad and type inside it while the user keeps working, read it back,
// screenshot, stop.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs Windows Sandbox enabled"]
async fn sb_live_work_inside_the_sandbox() {
    if std::env::var("XTRAOS_TEST_SANDBOX").ok().as_deref() != Some("1") {
        eprintln!("SKIP: enable Windows Sandbox, then set XTRAOS_TEST_SANDBOX=1");
        return;
    }
    let mut core = Core::start().await;
    let s = core.ok("sandbox", json!({"op": "start"})).await;
    eprintln!("SB-LIVE start: {}", s["text"]);
    let w = core
        .ok("sandbox", json!({"op": "call", "tool": "launch_app", "args": {"app": "notepad", "timeout_ms": 30000}, "timeout_ms": 60000}))
        .await;
    let win = w["ref"].as_str().unwrap().to_string();
    core.ok("sandbox", json!({"op": "call", "tool": "type", "args": {"window": win, "text": "typed inside the sandbox", "idle_ms": 0}}))
        .await;
    let r = core
        .ok(
            "sandbox",
            json!({"op": "call", "tool": "read_text", "args": {"window": win}}),
        )
        .await;
    assert!(
        r["text"]
            .as_str()
            .unwrap_or("")
            .contains("typed inside the sandbox"),
        "{r}"
    );
    let shot = core
        .ok(
            "sandbox",
            json!({"op": "call", "tool": "screenshot", "args": {"scale": 0.3}}),
        )
        .await;
    assert!(shot["png_base64"].as_str().is_some_and(|b| b.len() > 1000));
    core.ok("sandbox", json!({"op": "stop"})).await;
    eprintln!("SB-LIVE ok: worked inside the sandbox; the user's cursor was not used");
    core.stop().await;
}
