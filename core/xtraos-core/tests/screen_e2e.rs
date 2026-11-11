//! screen.find (ADR 0067), interactive: finds a testbed button among all open windows (the
//! user's own windows are only read), and the returned ref works at once.
mod common;

use common::{Core, Testbed};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens the testbed; reads every open window"]
async fn screen_find_across_windows() {
    let mut tb = Testbed::start(200, 160);
    let mut core = Core::start().await;
    let w = tb.window();

    let r = core.ok("run_skill", json!({"name": "screen.find", "args": {"query": "btnB", "timeout_ms": 15000, "max_windows": 40}})).await;
    let text = r["text"].as_str().unwrap();
    eprintln!("{text}");
    assert!(r["windows"].as_array().unwrap().iter().any(|x| x["window"] == w), "{r}");
    // The ref on the testbed's line clicks the button in the background.
    let block = text.split(&format!("{w} ")).nth(1).expect("testbed block");
    let line = block.lines().find(|l| l.contains("btnB")).expect("btnB line");
    let eref = line.split_whitespace().next().unwrap();
    assert!(eref.starts_with('e'), "{line}");
    let c = core.ok("click", json!({"ref": eref})).await;
    assert_eq!(c["mode"], "background", "{c}");
    assert_eq!(tb.state()["clicks"]["btnB"], 1);

    let none = core.ok("run_skill", json!({"name": "screen.find", "args": {"query": "zq-no-such-text-8317", "timeout_ms": 15000, "max_windows": 40}})).await;
    assert_eq!(none["windows"].as_array().unwrap().len(), 0, "{none}");
    assert!(none["searched"].as_u64().unwrap() >= 1, "{none}");
    assert!(none["text"].as_str().unwrap().contains("ocr"), "{none}");
    core.stop().await;
}
