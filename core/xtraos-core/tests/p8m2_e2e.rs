//! P8-M2 per-client profiles (ADR 0060), interactive: a real core with a test policy and
//! several agent clients on one testbed window.
mod common;

use std::path::PathBuf;

use common::{code, refs_by_name, Core, Testbed};
use serde_json::json;

fn exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_xtraos-core"))
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens the testbed; takes ~5 s"]
async fn p8m2_client_profiles() {
    let mut tb = Testbed::start(260, 180);
    let policy = json!({
        "clients": {
            "agent-ro": "read_only",
            "agent-bg": "background_only",
            "agent-no": "blocked",
            "agent-np": {"profile": "full", "apps": ["notepad.exe"]}
        }
    });
    let base = Core::start_process_with(&exe(), Some(&policy)).await;
    let w = tb.window();
    let mut ro = base.another("agent-ro").await;
    let mut bg = base.another("agent-bg").await;
    let mut no = base.another("agent-no").await;
    let mut np = base.another("agent-np").await;

    // read_only: reading works, changing is refused.
    let o = ro.ok("observe", json!({"window": w, "full": true})).await;
    let refs = refs_by_name(o["text"].as_str().unwrap());
    let e = ro.call("click", json!({"ref": refs["btnA"]})).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "{e}");
    assert!(e.to_string().contains("read_only"), "{e}");
    assert_eq!(ro.ok("status", json!({})).await["profile"], "read_only");

    // background_only: a click through the element works; a click at a point is refused.
    let r = bg.ok("click", json!({"ref": refs["btnA"]})).await;
    assert_eq!(r["mode"], "background", "{r}");
    let [x, y, cw, ch] = tb.rects["btnB"];
    let e = bg.call("click", json!({"window": w, "x": x + cw / 2, "y": y + ch / 2})).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "{e}");
    assert!(e.to_string().contains("background only"), "{e}");
    let e = bg.call("focus_window", json!({"window": w})).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "{e}");
    assert_eq!(bg.ok("status", json!({})).await["profile"], "background_only");

    // blocked: nothing but status.
    let e = no.call("observe", json!({"window": w})).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "{e}");
    assert_eq!(no.ok("status", json!({})).await["profile"], "blocked");

    // apps allowlist: full access, but only in Notepad.
    let e = np.call("click", json!({"ref": refs["btnB"]})).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "{e}");
    assert!(e.to_string().contains("notepad.exe"), "{e}");
    assert!(np.ok("status", json!({})).await["profile"].as_str().unwrap().contains("notepad.exe"));

    // An unlisted client keeps today's behaviour (new_clients defaults to full). The others
    // leave first: agent-bg's click leased the window (P8-M1). Refused calls leased nothing.
    drop((ro, bg, no, np));
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let mut base = base;
    assert_eq!(base.ok("status", json!({})).await["profile"], "full");
    let r = base.ok("click", json!({"ref": refs["btnB"]})).await;
    assert_eq!(r["mode"], "background", "{r}");

    let st = tb.state();
    assert_eq!(st["clicks"]["btnA"], 1, "only agent-bg's click on btnA went through: {st}");
    assert_eq!(st["clicks"]["btnB"], 1, "only the full client's click on btnB went through: {st}");
    base.stop().await;
}
