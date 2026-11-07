//! P2-M7 tests (docs/08 P2-LO-*, ADR 0034): off-screen and virtualised list rows, observe
//! speed on big windows. Automated, no human: they open the testbed and an Explorer window
//! (agent-opened, closed again with `cleanup`). `#[ignore]`: run via p2m7-gate.ps1 -Automated.
mod common;

use std::time::{Duration, Instant};

use common::{code, Core, Testbed};
use serde_json::{json, Value};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn text(v: &Value) -> String {
    v["text"].as_str().unwrap_or("").to_string()
}

fn median(mut v: Vec<u64>) -> u64 {
    v.sort_unstable();
    v[v.len() / 2]
}

async fn open_system32(core: &mut Core) -> String {
    let dir = format!(
        r"{}\System32",
        std::env::var("SystemRoot").unwrap_or(r"C:\Windows".into())
    );
    let w = core
        .ok(
            "launch_app",
            json!({"app": "explorer", "args": dir, "timeout_ms": 20000}),
        )
        .await;
    std::thread::sleep(Duration::from_millis(2500));
    w["ref"].as_str().unwrap().to_string()
}

// P2-LO-03: observe speed record on big windows (regression guard 1,000 ms p50). Parallel
// fetching was tried and is not faster (ADR 0035), so this records the serial numbers.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opens Explorer on System32"]
async fn lo03_observe_speed_record() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let ex = open_system32(&mut core).await;
    let tb = Testbed::start(900, 160);
    for (name, win) in [
        ("explorer System32", ex.clone()),
        ("testbed 200-row list", tb.window()),
    ] {
        let mut snap = Vec::new();
        let mut lines = 0;
        for i in 0..11 {
            let o = core
                .ok("observe", json!({"window": win, "full": true}))
                .await;
            if i > 0 {
                snap.push(o["ms"].as_u64().unwrap());
                lines = text(&o).lines().count();
            }
        }
        let p50 = median(snap);
        eprintln!("LO-03 {name}: observe p50 {p50} ms ({lines} lines)");
        assert!(p50 <= 1000, "{name}: regression guard");
    }
    let c = core.ok("cleanup", json!({})).await;
    eprintln!("cleanup: {}", text(&c));
    drop(tb);
    core.stop().await;
}

// P2-LO-01: off-screen rows of a long list are found and can be selected in the background.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opens the testbed"]
async fn lo01_offscreen_rows_found_and_selected() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(160, 160);
    let f = core
        .ok("find", json!({"window": tb.window(), "query": "item 150"}))
        .await;
    eprintln!("LO-01 find: {}", text(&f).replace('\n', " | "));
    assert_eq!(f["matches"], 1, "{f}");
    assert!(text(&f).contains("(offscreen)"), "{f}");
    let r = text(&f).split(' ').next().unwrap().to_string();
    let c = core.call("click", json!({"ref": r})).await;
    assert!(c.get("result").is_some(), "{c}");
    assert_eq!(
        c["result"]["via"], "select",
        "a row click selects (never opens): {c}"
    );
    let st = tb.state();
    eprintln!("LO-01 state after select: {st}");
    assert_eq!(st["listSel"], "Item 150", "Item 150 selected: {st}");
    drop(tb);
    core.stop().await;
}

// P2-LO-02: a virtualised Explorer row (never rendered) is found with deep: true, realised
// only when used, and selected.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opens Explorer on System32"]
async fn lo02_virtualized_rows_with_deep_find() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let ex = open_system32(&mut core).await;
    let plain = core
        .ok("find", json!({"window": ex, "query": "zipfldr.dll"}))
        .await;
    eprintln!("LO-02 plain find: {}", text(&plain).replace('\n', " | "));
    let t0 = Instant::now();
    let deep = core
        .ok(
            "find",
            json!({"window": ex, "query": "zipfldr.dll", "deep": true}),
        )
        .await;
    eprintln!(
        "LO-02 deep find ({} ms): {}",
        t0.elapsed().as_millis(),
        text(&deep).replace('\n', " | ")
    );
    assert!(deep["matches"].as_u64().unwrap_or(0) >= 1, "{deep}");
    let line = text(&deep)
        .lines()
        .find(|l| l.to_lowercase().contains("zipfldr.dll"))
        .unwrap()
        .to_string();
    let r = line.split(' ').next().unwrap().to_string();
    let c = core.call("click", json!({"ref": r})).await;
    eprintln!("LO-02 click: {c}");
    assert_eq!(
        c["result"]["via"], "select",
        "selects the file, never opens it: {c}"
    );
    let sel = core
        .ok("find", json!({"window": ex, "query": "zipfldr.dll"}))
        .await;
    eprintln!("LO-02 after: {}", text(&sel).replace('\n', " | "));
    assert!(
        text(&sel).contains("[selected]") || text(&sel).contains("selected"),
        "{sel}"
    );
    let partial = core
        .ok(
            "find",
            json!({"window": ex, "query": "kernel32", "deep": true}),
        )
        .await;
    eprintln!(
        "LO-02 partial deep: {}",
        text(&partial).replace('\n', " | ")
    );
    assert!(partial["matches"].as_u64().unwrap_or(0) >= 1, "{partial}");
    let bad = core
        .call("find", json!({"window": ex, "query": "x", "deep": "yes"}))
        .await;
    assert_eq!(code(&bad), "INVALID_ARGS");
    let c = core.ok("cleanup", json!({})).await;
    eprintln!("cleanup: {}", text(&c));
    core.stop().await;
}

#[link(name = "user32")]
extern "system" {
    fn FindWindowW(class: *const u16, title: *const u16) -> *mut std::ffi::c_void;
    fn GetDlgItem(h: *mut std::ffi::c_void, id: i32) -> *mut std::ffi::c_void;
    fn SendMessageW(h: *mut std::ffi::c_void, msg: u32, w: usize, l: isize) -> isize;
    fn PostMessageW(h: *mut std::ffi::c_void, msg: u32, w: usize, l: isize) -> i32;
}

// P2-LO-05: the read-only viewer shows the rules and the newest actions, writes nothing, and
// closes cleanly.
#[test]
#[ignore = "opens the viewer window"]
fn lo05_viewer_shows_rules_and_activity() {
    let home = std::env::temp_dir().join(format!("xtraos-p2m7-viewer-{}", common::rand_hex(3)));
    std::fs::create_dir_all(home.join("logs")).unwrap();
    std::fs::write(
        home.join("policy.json"),
        r#"{"classes": {"delete": "deny"}}"#,
    )
    .unwrap();
    std::fs::write(
        home.join("logs").join("actions-5000-1.jsonl"),
        "{\"ts_ms\":5000,\"method\":\"run_skill\",\"ok\":true,\"ms\":3.0,\"client\":\"test\"}\n",
    )
    .unwrap();
    let before = std::fs::read(home.join("policy.json")).unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_xtraos-core"))
        .arg("--viewer")
        .env("XTRAOS_HOME", &home)
        .spawn()
        .unwrap();
    let title: Vec<u16> = xtraos_core::viewer::TITLE
        .encode_utf16()
        .chain([0])
        .collect();
    let t0 = Instant::now();
    let w = loop {
        let w = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if !w.is_null() {
            break w;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "viewer window did not appear"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    std::thread::sleep(Duration::from_millis(300));
    let edit = unsafe { GetDlgItem(w, 10) };
    let mut buf = vec![0u16; 65536];
    let n = unsafe { SendMessageW(edit, 0x000D, buf.len(), buf.as_mut_ptr() as isize) }; // WM_GETTEXT
    let shown = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    eprintln!(
        "LO-05 viewer: {} chars; first lines: {}",
        shown.len(),
        shown.lines().take(3).collect::<Vec<_>>().join(" | ")
    );
    unsafe { PostMessageW(w, 0x0010, 0, 0) }; // WM_CLOSE
    let st = child.wait().unwrap();
    assert_eq!(st.code(), Some(0));
    assert!(shown.contains("delete               refused"), "{shown}");
    assert!(
        shown.contains("run_skill") && shown.contains("(test)"),
        "{shown}"
    );
    assert_eq!(
        std::fs::read(home.join("policy.json")).unwrap(),
        before,
        "viewer never writes"
    );
    let _ = std::fs::remove_dir_all(home);
}

// P2-LO-07: input census during 30 s of no use (hands off): what produces input events while
// nobody touches the PC. Counts only.
#[test]
#[ignore = "30 s, hands off"]
fn lo07_input_census_while_idle() {
    assert!(xtraos_core::safety::ensure_started());
    std::thread::sleep(Duration::from_millis(500));
    xtraos_core::safety::reset_input_census();
    std::thread::sleep(Duration::from_secs(30));
    let c = xtraos_core::safety::input_census();
    eprintln!("LO-07 census (30 s idle): {c}");
    assert_eq!(c["hooks"], true);
}
