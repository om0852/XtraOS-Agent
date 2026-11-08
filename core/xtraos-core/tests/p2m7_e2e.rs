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

// DG-01: a drag whose drop point (x,y) is on another item is refused before any input
// (TARGET_OCCUPIED), so nothing can land in a folder or the Recycle Bin by accident; observe
// with bounds gives the item's place.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opens the testbed"]
async fn dg01_drop_onto_an_item_is_refused() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let tb = Testbed::start(160, 160);
    let o = core
        .ok("observe", json!({"window": tb.window(), "bounds": true}))
        .await;
    let line = text(&o)
        .lines()
        .find(|l| l.contains("ListItem \"Item 2\""))
        .map(str::to_string)
        .unwrap_or_else(|| panic!("Item 2 with bounds: {o}"));
    let at = line.split(" @").nth(1).unwrap().split(' ').next().unwrap();
    let (x, y) = at.split_once(',').unwrap();
    let (x, y): (i32, i32) = (x.parse().unwrap(), y.parse().unwrap());
    let (bx, by) = tb.center("btnA");
    let r = core
        .call(
            "drag",
            json!({"from_x": bx, "from_y": by, "to_x": x, "to_y": y, "window": tb.window()}),
        )
        .await;
    eprintln!("DG-01: {}", r["error"]["message"]);
    assert_eq!(code(&r), "TARGET_OCCUPIED", "{r}");
    assert!(
        r["error"]["message"].as_str().unwrap().contains("Item 2"),
        "{r}"
    );
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

// The caller waits for the child after closing the window.
#[allow(clippy::zombie_processes)]
fn start_viewer(home: &std::path::Path) -> (std::process::Child, *mut std::ffi::c_void) {
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_xtraos-core"))
        .arg("--viewer")
        .env("XTRAOS_HOME", home)
        .spawn()
        .unwrap();
    let title: Vec<u16> = xtraos_core::viewer::TITLE
        .encode_utf16()
        .chain([0])
        .collect();
    let t0 = Instant::now();
    loop {
        let w = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
        if !w.is_null() {
            std::thread::sleep(Duration::from_millis(300));
            return (child, w);
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "viewer did not appear"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

// P2-ED-01: the user's editor saves a rule into policy.json, keeping keys it does not know.
#[test]
#[ignore = "opens the viewer window"]
fn ed01_editor_saves_rules() {
    use xtraos_core::policy::Class;
    let home = std::env::temp_dir().join(format!("xtraos-p2m8-ed-{}", common::rand_hex(3)));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("policy.json"),
        r#"{"note": "mine", "classes": {"delete": "confirm"}}"#,
    )
    .unwrap();
    let (mut child, w) = start_viewer(&home);
    let i = Class::ALL.iter().position(|c| *c == Class::Delete).unwrap() as i32;
    let cb = unsafe { GetDlgItem(w, 100 + i) };
    unsafe { SendMessageW(cb, 0x014E, 2, 0) }; // CB_SETCURSEL "refuse"
    let save = unsafe { GetDlgItem(w, 202) };
    unsafe { SendMessageW(save, 0x00F5, 0, 0) }; // BM_CLICK
    std::thread::sleep(Duration::from_millis(300));
    let mut buf = vec![0u16; 512];
    let st = unsafe { GetDlgItem(w, 203) };
    let n = unsafe { SendMessageW(st, 0x000D, buf.len(), buf.as_mut_ptr() as isize) };
    let status = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
    unsafe { PostMessageW(w, 0x0010, 0, 0) };
    assert_eq!(child.wait().unwrap().code(), Some(0));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.join("policy.json")).unwrap()).unwrap();
    eprintln!(
        "ED-01 status: {status}; delete = {}",
        v["classes"]["delete"]
    );
    assert!(status.starts_with("Saved."), "{status}");
    assert_eq!(v["classes"]["delete"], "deny");
    assert_eq!(v["note"], "mine", "unknown keys kept");
    assert_eq!(
        v["classes"]["credential_entry"], "deny",
        "hard rule written as is"
    );
    let _ = std::fs::remove_dir_all(home);
}

// P2-ED-03: the agent cannot read or drive any XtraOS window (the editor runs as its own
// xtraos-core.exe process).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opens the viewer window"]
async fn ed03_agent_cannot_touch_xtraos_windows() {
    let _s = SERIAL.lock().await;
    let home = std::env::temp_dir().join(format!("xtraos-p2m8-ed3-{}", common::rand_hex(3)));
    std::fs::create_dir_all(&home).unwrap();
    xtraos_core::safety::set_policy_path(home.join("policy-core.json"));
    let (mut child, w) = start_viewer(&home);
    let mut core = Core::start().await;
    let win = format!("w{}", w as usize);
    for (m, args) in [
        ("observe", json!({"window": win})),
        ("find", json!({"window": win, "query": "Save"})),
        ("close_window", json!({"window": win})),
    ] {
        let r = core.call(m, args).await;
        eprintln!("ED-03 {m}: {}", r["error"]["message"]);
        assert_eq!(code(&r), "BLOCKED_BY_POLICY", "{m}: {r}");
    }
    unsafe { PostMessageW(w, 0x0010, 0, 0) };
    assert_eq!(child.wait().unwrap().code(), Some(0));
    core.stop().await;
    let _ = std::fs::remove_dir_all(home);
}

// P3-FD-01..03: frame_diff on the testbed: baseline, no change, a text change found where
// the text field is, and wait_ms catching a delayed change.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opens the testbed"]
async fn fd01_03_frame_diff() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(160, 160);
    let w = tb.window();
    let b = core.ok("frame_diff", json!({"window": w})).await;
    assert_eq!(b["baseline"], true, "{b}");
    let n = core.ok("frame_diff", json!({"window": w})).await;
    eprintln!("FD-01 still: {}", text(&n));
    assert_eq!(n["changed"], false, "{n}");
    tb.cmd("settext frame diff test 1234567890");
    std::thread::sleep(Duration::from_millis(300));
    let c = core.ok("frame_diff", json!({"window": w})).await;
    eprintln!("FD-02 after settext: {}", text(&c));
    assert_eq!(c["changed"], true, "{c}");
    let [fx, fy, fw, fh] = tb.rects["text"];
    let hit = c["regions"].as_array().unwrap().iter().any(|r| {
        let (x, y, rw, rh) = (
            r["x"].as_i64().unwrap() as i32,
            r["y"].as_i64().unwrap() as i32,
            r["w"].as_i64().unwrap() as i32,
            r["h"].as_i64().unwrap() as i32,
        );
        x < fx + fw && fx < x + rw && y < fy + fh && fy < y + rh
    });
    assert!(
        hit,
        "a changed region covers the text field {:?}: {c}",
        tb.rects["text"]
    );
    tb.cmd("latertext 600 changed later");
    let t0 = Instant::now();
    let wt = core
        .ok("frame_diff", json!({"window": w, "wait_ms": 5000}))
        .await;
    let took = t0.elapsed().as_millis();
    eprintln!("FD-03 wait: {} after {took} ms", text(&wt));
    assert_eq!(wt["changed"], true, "{wt}");
    assert!(
        (400..4000).contains(&took),
        "returned when the change came: {took} ms"
    );
    let q = core
        .ok("frame_diff", json!({"window": w, "wait_ms": 400}))
        .await;
    assert_eq!(q["changed"], false, "{q}");
    drop(tb);
    core.stop().await;
}

fn rr_home() -> std::path::PathBuf {
    let home = std::env::temp_dir().join(format!("xtraos-p3m2-home-{}", std::process::id()));
    std::fs::create_dir_all(home.join("skills")).unwrap();
    std::env::set_var("XTRAOS_HOME", &home);
    xtraos_core::safety::set_policy_path(home.join("policy.json"));
    home
}

// P3-RR-01..04: record actions on the testbed, replay them after the window moved (locators,
// not refs), replay with a changed argument, fail cleanly when the window is gone; password
// actions are never recorded; name and state rules.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opens the testbed"]
async fn rr01_04_record_and_replay() {
    let _s = SERIAL.lock().await;
    let home = rr_home();
    let mut core = Core::start().await;
    let mut tb = Testbed::start(160, 160);
    let w = tb.window();

    let bad = core
        .call("record_session", json!({"op": "start", "name": "../evil"}))
        .await;
    assert_eq!(code(&bad), "INVALID_ARGS");
    core.ok("record_session", json!({"op": "start", "name": "rr-test"}))
        .await;
    let again = core
        .call("record_session", json!({"op": "start", "name": "other"}))
        .await;
    assert_eq!(code(&again), "INVALID_ARGS", "one recording at a time");
    let o = core.ok("observe", json!({"window": w, "full": true})).await;
    let refs = common::refs_by_name(&text(&o));
    core.ok("click", json!({"ref": refs["btnA"]})).await;
    core.ok(
        "set_value",
        json!({"ref": refs["text"], "value": "hello rec"}),
    )
    .await;
    let pw = core
        .call(
            "set_value",
            json!({"ref": refs["password"], "value": "secret-pw"}),
        )
        .await;
    assert_eq!(
        code(&pw),
        "BLOCKED_BY_POLICY",
        "password entry refused: {pw}"
    );
    let st = core.ok("record_session", json!({"op": "status"})).await;
    eprintln!("RR status: {}", text(&st));
    assert_eq!(
        st["steps"], 2,
        "observe not recorded, refused action not recorded"
    );
    let saved = core.ok("record_session", json!({"op": "stop"})).await;
    eprintln!("RR-01 saved: {}", text(&saved));
    let file = std::fs::read_to_string(home.join("skills").join("rr-test.json")).unwrap();
    assert!(!file.contains("secret-pw"), "password text never stored");
    let def: Value = serde_json::from_str(&file).unwrap();
    assert_eq!(
        def["steps"][0]["args"]["ref"]["locate"]["name"], "btnA",
        "{def}"
    );
    assert_eq!(
        def["steps"][0]["args"]["ref"]["locate"]["process"],
        "testbed.exe"
    );
    assert_eq!(def["args"]["value1"]["default"], "hello rec", "{def}");
    assert_eq!(def["steps"][1]["args"]["value"], "${value1}");

    // Move the window and reset it: refs are gone, locators still work.
    tb.cmd("reset");
    core.ok("window_bounds", json!({"window": w, "x": 420, "y": 220}))
        .await;
    let r = core.ok("run_skill", json!({"name": "rr-test"})).await;
    eprintln!(
        "RR-01 replay: {}",
        r["text"].as_str().unwrap_or(&r.to_string())
    );
    let s1 = tb.state();
    assert_eq!(s1["clicks"]["btnA"], 1, "{s1}");
    assert_eq!(s1["text"], "hello rec", "{s1}");
    let r2 = core
        .ok(
            "run_skill",
            json!({"name": "rr-test", "args": {"value1": "changed text"}}),
        )
        .await;
    assert!(r2.get("ok").is_none_or(|o| o == true), "{r2}");
    let s2 = tb.state();
    assert_eq!(s2["clicks"]["btnA"], 2);
    assert_eq!(s2["text"], "changed text", "argument override");
    eprintln!("RR-02 ok: replay after move + argument override");

    drop(tb);
    std::thread::sleep(Duration::from_millis(500));
    let gone = core.call("run_skill", json!({"name": "rr-test"})).await;
    eprintln!("RR-03 window gone: {gone}");
    let first = &gone["result"]["steps"][0];
    assert_eq!(first["ok"], false, "{gone}");
    assert_eq!(first["error"]["code"], "WINDOW_NOT_FOUND", "{gone}");

    core.ok(
        "record_session",
        json!({"op": "start", "name": "empty-one"}),
    )
    .await;
    let e = core.ok("record_session", json!({"op": "stop"})).await;
    assert!(text(&e).contains("nothing saved"), "{e}");
    assert!(!home.join("skills").join("empty-one.json").exists());
    eprintln!("RR-04 ok: name rules, one at a time, empty recordings not saved");
    core.stop().await;
    let _ = std::fs::remove_dir_all(home);
}

// LG-01 (ADR 0038): `--verify-log` passes on an intact log and finds a one-character edit.
#[test]
fn lg01_verify_log_cli() {
    use xtraos_core::actionlog::{ActionLog, Entry};
    let home = std::env::temp_dir().join(format!("xtraos-lg-{}", common::rand_hex(3)));
    let logs = home.join("logs");
    let log = ActionLog::create(&logs).unwrap();
    for m in ["status", "observe", "click"] {
        log.record(&Entry {
            ts_ms: 1,
            session: Some("s1"),
            client: Some("t"),
            method: m,
            ok: true,
            code: None,
            ms: 2.5,
            policy: Some("allow"),
        });
    }
    let path = log.path().to_path_buf();
    drop(log);
    let run = || {
        std::process::Command::new(env!("CARGO_BIN_EXE_xtraos-core"))
            .arg("--verify-log")
            .env("XTRAOS_HOME", &home)
            .output()
            .unwrap()
    };
    let ok = run();
    let out = String::from_utf8_lossy(&ok.stdout).to_string();
    eprintln!("LG-01 intact: {out}");
    assert_eq!(ok.status.code(), Some(0));
    assert!(out.starts_with("OK: 3 entries"), "{out}");
    let t = std::fs::read_to_string(&path)
        .unwrap()
        .replace("\"observe\"", "\"0bserve\"");
    std::fs::write(&path, t).unwrap();
    let bad = run();
    let out = String::from_utf8_lossy(&bad.stdout).to_string();
    eprintln!("LG-01 edited: {out}");
    assert_eq!(bad.status.code(), Some(1));
    assert!(
        out.contains("line 2") && out.contains("content changed"),
        "{out}"
    );
    let _ = std::fs::remove_dir_all(home);
}

#[link(name = "user32")]
extern "system" {
    fn GetWindowTextW(h: *mut std::ffi::c_void, buf: *mut u16, n: i32) -> i32;
}

fn answer_next_dialog(i: i32) -> std::thread::JoinHandle<Option<String>> {
    std::thread::spawn(move || {
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(10) {
            if let Some(&d) = xtraos_core::safety::prompt_windows().first() {
                std::thread::sleep(Duration::from_millis(300));
                let mut buf = [0u16; 2048];
                let st = unsafe { GetDlgItem(d as _, xtraos_core::safety::PROMPT_TEXT_ID) };
                let n = unsafe { GetWindowTextW(st, buf.as_mut_ptr(), 2048) };
                xtraos_core::safety::answer_prompt_for_tests(i as usize);
                return Some(String::from_utf16_lossy(&buf[..n.max(0) as usize]));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    })
}

// TL-01 (ADR 0038): max_actions_per_task = 5. The 6th action asks; Stop refuses (and keeps
// refusing while the agent stays busy); Resume from the tray resets; Continue carries on.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "shows a core dialog (answered by the test)"]
async fn tl01_max_actions_per_task() {
    let _s = SERIAL.lock().await;
    let home = std::env::temp_dir().join(format!("xtraos-tl-{}", common::rand_hex(3)));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("policy.json"),
        r#"{"max_actions_per_task": 5, "task_idle_minutes": 10}"#,
    )
    .unwrap();
    xtraos_core::safety::set_policy_path(home.join("policy.json"));
    let mut core = Core::start().await;
    // Policy file applied (a first load may ask: approve it).
    let first = answer_next_dialog(0);
    let r = core.call("list_windows", json!({})).await;
    let _ = first.join();
    assert!(r.get("result").is_some(), "{r}");
    for _ in 0..4 {
        core.ok("list_windows", json!({})).await;
    }
    let human = answer_next_dialog(1); // Stop
    let r = core.call("list_windows", json!({})).await;
    let msg = human.join().unwrap().expect("long-task dialog");
    eprintln!("TL-01 dialog: {}", msg.replace('\n', " "));
    assert!(msg.contains("5 actions in this task"), "{msg}");
    assert_eq!(code(&r), "BLOCKED_BY_POLICY", "{r}");
    let r = core.call("list_windows", json!({})).await;
    assert_eq!(
        code(&r),
        "BLOCKED_BY_POLICY",
        "still refused while busy: {r}"
    );
    assert!(
        xtraos_core::safety::prompt_windows().is_empty(),
        "no dialog spam"
    );
    // Resume from the tray resets the task.
    xtraos_core::safety::resume_for_tests();
    std::thread::sleep(Duration::from_millis(200));
    for _ in 0..5 {
        core.ok("list_windows", json!({})).await;
    }
    let human = answer_next_dialog(0); // Continue
    let r = core.call("list_windows", json!({})).await;
    human.join().unwrap().expect("dialog again at the limit");
    assert!(r.get("result").is_some(), "Continue lets it go on: {r}");
    core.ok("list_windows", json!({})).await;
    eprintln!("TL-01 ok: ask at the limit, Stop refuses, Resume resets, Continue carries on");
    core.stop().await;
    let _ = std::fs::remove_dir_all(home);
}
