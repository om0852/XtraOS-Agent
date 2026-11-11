//! P7-M3 slice 2 (ADR 0053), interactive: a "reproduce this bug" report on the testbed.
mod common;

use common::{Core, Testbed};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens the testbed, writes a report under Documents\\XtraOS reports"]
async fn p7m3_bug_report_with_a_failed_step_and_a_note() {
    let mut tb = Testbed::start(260, 180);
    let mut core = Core::start().await;
    let w = tb.window();
    let name = format!("p7m3-test-{}", common::rand_hex(4));

    let s = core
        .ok("record_session", json!({"op": "start", "name": name, "report": true}))
        .await;
    assert!(s["text"].as_str().unwrap().contains("bug report in"), "{s}");

    let o = core.ok("observe", json!({"window": w, "full": true})).await;
    let refs = common::refs_by_name(o["text"].as_str().unwrap());
    core.ok("click", json!({"ref": refs["btnA"]})).await;
    core.ok("set_value", json!({"ref": refs["text"], "value": "hello report"})).await;
    // A failing step: a ref that does not exist.
    let e = core.call("click", json!({"ref": "e999999"})).await;
    assert!(e.get("error").is_some(), "{e}");
    core.ok("record_session", json!({"op": "note", "text": "btnA should turn green but stays grey"}))
        .await;

    let r = core
        .ok("record_session", json!({"op": "stop", "overwrite": true}))
        .await;
    let report = r["report"].as_str().expect("report path").to_string();
    let md = std::fs::read_to_string(&report).unwrap();
    eprintln!("{md}");
    assert!(md.contains("- Steps: 3 (1 failed)"), "{md}");
    assert!(md.contains("click Button \"btnA\""), "{md}");
    assert!(md.contains("set value \"hello report\""), "{md}");
    assert!(md.contains("**FAILED**: STALE_REF"), "{md}");
    assert!(md.contains("btnA should turn green"), "{md}");
    // Only the app under test is pictured and named — never whatever else was in front.
    let apps = md.lines().find(|l| l.starts_with("- Apps:")).unwrap_or("");
    assert_eq!(apps, "- Apps: testbed.exe \"XtraOS Testbed\"", "{md}");
    assert!(md.contains(&format!("run_skill {{\"name\": \"{name}\"}}")), "{md}");
    let dir = std::path::Path::new(&report).parent().unwrap().to_path_buf();
    let pngs = std::fs::read_dir(&dir)
        .unwrap()
        .filter(|e| e.as_ref().unwrap().path().extension().is_some_and(|x| x == "png"))
        .count();
    assert!(pngs >= 3, "screenshots: {pngs}");
    assert_eq!(tb.state()["text"], "hello report");

    // Clean up: the report folder and the saved skill (test artefacts).
    let _ = std::fs::remove_dir_all(&dir);
    if let Some(p) = r["path"].as_str() {
        let _ = std::fs::remove_file(p);
    }
    core.stop().await;
}
