//! P8-M1 several clients at once (ADR 0059), interactive: two agent sessions on one core and
//! one testbed window.
mod common;

use std::time::{Duration, Instant};

use common::{code, refs_by_name, Core, Testbed};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens the testbed; takes ~12 s (one queue wait)"]
async fn p8m1_two_agents_one_window() {
    let mut tb = Testbed::start(260, 180);
    let base = Core::start().await;
    let mut a = base.another("agent-a").await;
    let mut b = base.another("agent-b").await;
    let w = tb.window();

    let o = a.ok("observe", json!({"window": w, "full": true})).await;
    let refs = refs_by_name(o["text"].as_str().unwrap());
    let r = a.ok("click", json!({"ref": refs["btnA"]})).await;
    assert_eq!(r["mode"], "background", "{r}");

    // B can read the window while A holds it.
    let ob = b.ok("observe", json!({"window": w, "full": true})).await;
    assert!(ob["text"].as_str().unwrap().contains("btnB"));
    // status shows the lease.
    let s = b.ok("status", json!({})).await;
    let locks = s["locks"].as_array().unwrap();
    assert_eq!(locks.len(), 1, "{s}");
    assert_eq!(locks[0]["client"], "agent-a", "{s}");
    assert_eq!(locks[0]["window"], w, "{s}");

    // B's click waits in the queue, then WINDOW_BUSY naming A.
    let t0 = Instant::now();
    let e = b.call("click", json!({"ref": refs["btnB"]})).await;
    assert_eq!(code(&e), "WINDOW_BUSY", "{e}");
    assert!(t0.elapsed() >= Duration::from_secs(9), "waited in the queue first");
    assert!(e.to_string().contains("agent-a"), "{e}");
    assert_eq!(tb.state()["clicks"]["btnB"], 0, "nothing was clicked");

    // A leaves: its lease goes at once, and B's click goes through.
    drop(a);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let r = b.ok("click", json!({"ref": refs["btnB"]})).await;
    assert_eq!(r["mode"], "background", "{r}");
    let st = tb.state();
    assert_eq!(st["clicks"]["btnA"], 1, "{st}");
    assert_eq!(st["clicks"]["btnB"], 1, "{st}");
    let s = b.ok("status", json!({})).await;
    assert_eq!(s["locks"][0]["client"], "agent-b", "{s}");
    drop(b);
    base.stop().await;
}
