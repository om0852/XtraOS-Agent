//! P8-M3 activity dashboard (ADR 0061), interactive: two agents on one core; the user pauses
//! one (as the tray menu does), the other keeps working.
mod common;

use std::time::{Duration, Instant};

use common::{code, refs_by_name, Core, Testbed};
use serde_json::json;
use xtraos_core::agents;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens the testbed; takes ~5 s"]
async fn p8m3_pause_one_agent() {
    let mut tb = Testbed::start(260, 180);
    let base = Core::start().await;
    let mut a = base.another("agent-pause-a").await;
    let mut b = base.another("agent-pause-b").await;
    let w = tb.window();

    let o = a.ok("observe", json!({"window": w, "full": true})).await;
    let refs = refs_by_name(o["text"].as_str().unwrap());
    b.ok("list_windows", json!({})).await;

    // status.agents: both, with their last action (method only).
    let s = b.ok("status", json!({})).await;
    let list = s["agents"].as_array().unwrap();
    let find = |c: &str| list.iter().find(|x| x["client"] == c).cloned().unwrap_or_default();
    assert_eq!(find("agent-pause-a")["last_action"], "observe", "{s}");
    assert_eq!(find("agent-pause-b")["last_action"], "list_windows", "{s}");
    assert_eq!(find("agent-pause-a")["connected"], true, "{s}");

    // A is waiting for something; the user pauses A: the running call stops at once.
    let mut a2 = base.another("agent-pause-a").await;
    let waiting = tokio::spawn(async move {
        let t0 = Instant::now();
        let r = a2.call("wait_for", json!({"window_title": "never-appears-xyz", "timeout_ms": 15000})).await;
        (r, t0.elapsed())
    });
    tokio::time::sleep(Duration::from_millis(1000)).await;
    agents::set_paused("agent-pause-a", true);
    let (r, took) = waiting.await.unwrap();
    assert_eq!(code(&r), "STOPPED", "{r}");
    assert!(r.to_string().contains("paused this agent"), "{r}");
    assert!(took < Duration::from_secs(5), "stopped mid-call, took {took:?}");

    // While paused: A's calls are STOPPED, its status says paused; B works on.
    let e = a.call("click", json!({"ref": refs["btnA"]})).await;
    assert_eq!(code(&e), "STOPPED", "{e}");
    assert_eq!(a.ok("status", json!({})).await["state"], "paused");
    let r = b.ok("click", json!({"ref": refs["btnB"]})).await;
    assert_eq!(r["mode"], "background", "{r}");
    assert_eq!(b.ok("status", json!({})).await["state"], "running");
    let s = b.ok("status", json!({})).await;
    assert!(s["agents"].as_array().unwrap().iter().any(|x| x["client"] == "agent-pause-a" && x["paused"] == true), "{s}");
    assert!(s["agents"].as_array().unwrap().iter().any(|x| x["client"] == "agent-pause-a" && x["refused"].as_u64() >= Some(1)), "{s}");

    // Resumed: A acts again (after B's lease on the window ends — B leaves).
    agents::set_paused("agent-pause-a", false);
    drop(b);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let r = a.ok("click", json!({"ref": refs["btnA"]})).await;
    assert_eq!(r["mode"], "background", "{r}");
    let st = tb.state();
    assert_eq!(st["clicks"]["btnA"], 1, "A's paused click never happened: {st}");
    assert_eq!(st["clicks"]["btnB"], 1, "{st}");

    // An ended session is still listed (10 min), as ended.
    let s = a.ok("status", json!({})).await;
    assert!(s["agents"].as_array().unwrap().iter().any(|x| x["client"] == "agent-pause-b" && x["connected"] == false), "{s}");
    drop(a);
    base.stop().await;
}
