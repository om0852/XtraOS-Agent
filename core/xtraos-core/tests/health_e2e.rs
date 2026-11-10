//! System health guard (ADR 0050), interactive: real readings, the warning on results, and the
//! brake when the agent keeps opening windows. Uses lowered limits (warn at 2 windows opened in
//! 10 minutes, critical at 4) and a 3 s prompt: the brake prompt shows briefly on screen and,
//! unanswered, counts as "Stop it".
mod common;

use common::{code, Core};
use serde_json::json;

fn testbed_exe() -> String {
    common::testbed_exe().display().to_string()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens the testbed 5 times; a brake prompt shows for 3 s"]
async fn health01_readings_warning_and_brake() {
    std::env::set_var("XTRAOS_HEALTH_OPENED_WARN", "2");
    std::env::set_var("XTRAOS_HEALTH_OPENED_CRIT", "4");
    std::env::set_var("XTRAOS_HEALTH_PROMPT_MS", "3000");
    let mut core = Core::start().await;

    // Readings: no admin needed.
    let h = core.ok("system", json!({"op": "health"})).await;
    assert!(h["ram"]["total_gb"].as_f64().unwrap() > 0.5, "{h}");
    assert!(h["cpu_pct"].as_f64().is_some(), "{h}");
    assert!(!h["disks"].as_array().unwrap().is_empty(), "{h}");
    assert!(!h["top"].as_array().unwrap().is_empty(), "{h}");
    eprintln!("{}", h["text"].as_str().unwrap());
    let s = core.ok("status", json!({})).await;
    assert!(s["health"]["ram_pct"].as_f64().is_some(), "{s}");

    // 1st, 2nd window: quiet. 3rd: the result warns (2 already opened, limit 2).
    let launch = json!({"app": testbed_exe(), "args": ["300", "200"], "timeout_ms": 15000, "reuse": false});
    let a = core.ok("launch_app", launch.clone()).await;
    assert!(a.get("health_warning").is_none(), "{a}");
    core.ok("launch_app", launch.clone()).await;
    let c = core.ok("launch_app", launch.clone()).await;
    let w = c["health_warning"].as_str().unwrap_or_default();
    assert!(w.contains("opened 3 windows"), "{c}");
    // Every tool result carries it, not only opening ones.
    let l = core.ok("list_windows", json!({})).await;
    assert!(l["health_warning"].is_string(), "{l}");

    // 4 opened = critical: the 5th opening asks the user; unanswered → blocked.
    core.ok("launch_app", launch.clone()).await;
    let e = core.call("launch_app", launch.clone()).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "{e}");
    assert!(e.to_string().contains("under strain"), "{e}");

    // Reading and closing are never braked: cleanup closes only what the agent opened.
    let done = core.ok("cleanup", json!({})).await;
    eprintln!("cleanup: {done}");
    core.stop().await;
}
