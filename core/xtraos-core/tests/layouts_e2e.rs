//! Window layouts (ADR 0065), interactive: two testbed windows, a private XtraOS home.
mod common;

use common::{Core, Testbed};
use serde_json::{json, Value};

/// The testbed windows now: (ref, x, y, w, h, state).
async fn testbeds(core: &mut Core) -> Vec<(String, i64, i64, i64, i64, String)> {
    let l = core.ok("list_windows", json!({"filter": "testbed"})).await;
    l["windows"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["process"].as_str().unwrap_or("").eq_ignore_ascii_case("testbed.exe"))
        .map(|w: &Value| {
            (
                w["ref"].as_str().unwrap().to_string(),
                w["x"].as_i64().unwrap(),
                w["y"].as_i64().unwrap(),
                w["w"].as_i64().unwrap(),
                w["h"].as_i64().unwrap(),
                w["state"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens two testbed windows, moves and minimizes them"]
async fn layouts_save_and_restore() {
    let home = std::env::temp_dir().join(format!("xtraos-layouts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: set before the core starts and before any other thread reads it.
    unsafe { std::env::set_var("XTRAOS_HOME", &home) };
    let mut core = Core::start().await;
    let mut a = Testbed::start(120, 140);
    let mut b = Testbed::start(560, 180);
    let (wa, wb) = (a.window(), b.window());

    // The arrangement to remember: A normal at a known place and size, B minimized.
    core.ok("window_bounds", json!({"window": wa, "x": 150, "y": 160, "w": 420, "h": 330})).await;
    core.ok("window_bounds", json!({"window": wb, "state": "minimize"})).await;
    let s = core.ok("run_skill", json!({"name": "layout.save", "args": {"name": "Test Layout", "filter": "testbed"}})).await;
    assert_eq!(s["windows"], 2, "{s}");

    // Mess it up: A moved and resized, B restored.
    core.ok("window_bounds", json!({"window": wa, "x": 700, "y": 420, "w": 300, "h": 260})).await;
    core.ok("window_bounds", json!({"window": wb, "state": "restore"})).await;

    let r = core.ok("run_skill", json!({"name": "layout.restore", "args": {"name": "test layout"}})).await;
    eprintln!("{}", r["text"]);
    assert_eq!(r["placed"], 2, "{r}");
    let now = testbeds(&mut core).await;
    assert_eq!(now.len(), 2, "{now:?}");
    // Both windows share a title, so either may take either place: one is back at the saved
    // place and size, the other is minimized again.
    let normal: Vec<_> = now.iter().filter(|w| w.5 == "normal").collect();
    let mini: Vec<_> = now.iter().filter(|w| w.5 == "minimized").collect();
    assert_eq!((normal.len(), mini.len()), (1, 1), "{now:?}");
    let n = normal[0];
    assert!((n.1 - 150).abs() <= 3 && (n.2 - 160).abs() <= 3 && (n.3 - 420).abs() <= 3 && (n.4 - 330).abs() <= 3, "{n:?}");

    // A saved window that is not open is reported, not launched.
    let f = home.join("layouts").join("test_layout.json");
    let mut doc: Value = serde_json::from_str(&std::fs::read_to_string(&f).unwrap()).unwrap();
    doc["windows"].as_array_mut().unwrap().push(json!({"process": "nosuchapp.exe", "title": "Gone", "rect": [0, 0, 300, 300], "state": "normal"}));
    std::fs::write(&f, doc.to_string()).unwrap();
    let r = core.ok("run_skill", json!({"name": "layout.restore", "args": {"name": "test layout"}})).await;
    assert!(r["text"].as_str().unwrap().contains("not open") && r["text"].as_str().unwrap().contains("nosuchapp.exe"), "{r}");

    let l = core.ok("run_skill", json!({"name": "layout.list"})).await;
    assert!(l["text"].as_str().unwrap().contains("test layout: 3 window(s)"), "{l}");
    core.ok("run_skill", json!({"name": "layout.delete", "args": {"name": "test layout"}})).await;
    let e = core.call("run_skill", json!({"name": "layout.restore", "args": {"name": "test layout"}})).await;
    assert_eq!(common::code(&e), "INVALID_ARGS", "{e}");
    drop((a, b));
    core.stop().await;
    let _ = std::fs::remove_dir_all(&home);
}
