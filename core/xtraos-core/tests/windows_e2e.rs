//! M3 interactive tests (docs/08-test-plan.md P1-WIN-*). They open/move/close windows, so they
//! are `#[ignore]`; run via tests/phase1/m3-gate.ps1 -Interactive.
//! Opt-in extras (env vars): XTRAOS_TEST_UAC=1 (needs a human to click Yes on UAC) and
//! XTRAOS_TEST_BROWSERS=1 (opens new Chrome/Firefox windows, searches, closes them).
mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{code, Core};
use serde_json::{json, Value};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn testbed_exe() -> String {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testbed/bin/testbed.exe");
    assert!(p.exists(), "build the testbed first: testbed/build.ps1");
    std::fs::canonicalize(p)
        .unwrap()
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string()
}

async fn windows(core: &mut Core, filter: &str) -> Vec<Value> {
    core.ok("list_windows", json!({"filter": filter})).await["windows"]
        .as_array()
        .unwrap()
        .clone()
}

fn near(a: &Value, b: i64) -> bool {
    (a.as_i64().unwrap() - b).abs() <= 1
}

/// Ends a test-launched testbed process if the test fails midway (never used on user apps).
struct EndTestbedOnDrop(u64);

impl Drop for EndTestbedOnDrop {
    fn drop(&mut self) {
        let _ = std::process::Command::new("taskkill")
            .args(["/PID", &self.0.to_string(), "/F"])
            .output();
    }
}

// P1-WIN-01/02/03/04/05 on the testbed launched like any normal app.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens, moves and closes a testbed window"]
async fn win01_launch_list_bounds_states_focus_close() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;

    // launch_app returns the NEW window
    let w = core
        .ok(
            "launch_app",
            json!({"app": testbed_exe(), "args": ["300", "200"], "timeout_ms": 15000}),
        )
        .await;
    let r = w["ref"].as_str().unwrap().to_string();
    assert_eq!(w["process"], "testbed.exe", "{w}");
    let _cleanup = EndTestbedOnDrop(w["pid"].as_u64().unwrap());
    assert_eq!(w["title"], "XtraOS Testbed", "{w}");
    assert_eq!(w["elevated"], false);

    // list_windows contains it (filter by title)
    let found = windows(&mut core, "xtraos testbed").await;
    assert!(
        found.iter().any(|x| x["ref"] == r),
        "launched window missing from list: {found:?}"
    );

    // set visible-frame bounds precisely
    let b = core
        .ok(
            "window_bounds",
            json!({"window": r, "x": 260, "y": 180, "w": 700, "h": 480}),
        )
        .await;
    assert!(
        near(&b["x"], 260) && near(&b["y"], 180) && near(&b["w"], 700) && near(&b["h"], 480),
        "{b}"
    );
    assert_eq!(b["state"], "normal");

    // states
    let m = core
        .ok("window_bounds", json!({"window": r, "state": "maximize"}))
        .await;
    assert_eq!(m["state"], "maximized", "{m}");
    let n = core
        .ok("window_bounds", json!({"window": r, "state": "restore"}))
        .await;
    assert_eq!(n["state"], "normal", "{n}");
    assert!(
        near(&n["x"], 260) && near(&n["w"], 700),
        "restore returns to the old bounds: {n}"
    );
    let mn = core
        .ok("window_bounds", json!({"window": r, "state": "minimize"}))
        .await;
    assert_eq!(mn["state"], "minimized", "{mn}");

    // focus restores from minimised and brings to front
    core.ok("focus_window", json!({"window": r})).await;
    let now = windows(&mut core, "xtraos testbed").await;
    let me = now.iter().find(|x| x["ref"] == r).unwrap();
    assert_eq!(me["state"], "normal", "{me}");
    assert_eq!(me["focused"], true, "{me}");

    // graceful close
    core.ok("close_window", json!({"window": r})).await;
    let after = windows(&mut core, "xtraos testbed").await;
    assert!(
        !after.iter().any(|x| x["ref"] == r),
        "window still listed after close"
    );
    let r2 = core.call("focus_window", json!({"window": r})).await;
    assert_eq!(code(&r2), "WINDOW_NOT_FOUND", "{r2}");
    core.stop().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn win02_unknown_app_and_bad_refs() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let r = core
        .call("launch_app", json!({"app": "definitely-not-an-app-xyz"}))
        .await;
    assert_eq!(code(&r), "INVALID_ARGS", "{r}");
    let r = core
        .call("close_window", json!({"window": "w999999999"}))
        .await;
    assert_eq!(code(&r), "WINDOW_NOT_FOUND", "{r}");
    core.stop().await;
}

// P1-WIN-07 (part 2): any elevated window that is open (e.g. the one win03 created) is
// refused untouched. Runs after win03 (alphabetical order with --test-threads=1).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn win04_elevated_windows_are_refused_before_any_input() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    if xtraos_core::winmgr::core_is_elevated() {
        eprintln!("SKIP: the test process itself is elevated");
        core.stop().await;
        return;
    }
    let all = windows(&mut core, "").await;
    let Some(admin) = all.iter().find(|w| w["elevated"] == true).cloned() else {
        eprintln!("SKIP: no elevated window is open (run with XTRAOS_TEST_UAC=1 to create one)");
        core.stop().await;
        return;
    };
    eprintln!(
        "using elevated window {} ({})",
        admin["ref"], admin["process"]
    );
    let (x, y) = (
        admin["x"].as_i64().unwrap() + 20,
        admin["y"].as_i64().unwrap() + 20,
    );
    for (m, p) in [
        ("click", json!({"x": x, "y": y, "window": admin["ref"]})),
        ("type", json!({"window": admin["ref"], "text": "nope"})),
        ("close_window", json!({"window": admin["ref"]})),
        (
            "window_bounds",
            json!({"window": admin["ref"], "state": "minimize"}),
        ),
    ] {
        let r = core.call(m, p).await;
        assert_eq!(code(&r), "ELEVATION_REQUIRED", "{m}: {r}");
    }
    core.stop().await;
}

// P1-WIN-07 (part 1, human): launch an admin cmd via UAC, then input is refused. The admin
// window stays open for win04 and is closed by the user afterwards.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "human: click Yes on the UAC prompt"]
async fn win03_uac_launch_then_input_requires_elevation() {
    if std::env::var("XTRAOS_TEST_UAC").as_deref() != Ok("1") {
        eprintln!("SKIP: set XTRAOS_TEST_UAC=1 to run (needs a human UAC click)");
        return;
    }
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    eprintln!(">>> XtraOS asks first (M5 policy: elevated = confirm): click \"Allow once\"; then the Windows admin (UAC) prompt: click Yes.");
    let w = core
        .ok(
            "launch_app",
            json!({"app": "cmd", "elevated": true, "timeout_ms": 60000}),
        )
        .await;
    assert_eq!(w["elevated"], true, "{w}");
    let r = core
        .call("type", json!({"window": w["ref"], "text": "echo hi"}))
        .await;
    assert_eq!(code(&r), "ELEVATION_REQUIRED", "{r}");
    eprintln!(
        ">>> Please close the Administrator Command Prompt ({}) yourself.",
        w["ref"]
    );
    core.stop().await;
}

async fn browser_search(core: &mut Core, app: &str, args: Value, query: &str) {
    let w = core
        .ok(
            "launch_app",
            json!({"app": app, "args": args, "timeout_ms": 20000}),
        )
        .await;
    let r = w["ref"].clone();
    core.ok("focus_window", json!({"window": r})).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    core.ok("press_keys", json!({"window": r, "keys": ["Ctrl+L"]}))
        .await;
    core.ok("type", json!({"window": r, "text": query, "submit": true}))
        .await;
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let ws = windows(core, "").await;
        let title = ws
            .iter()
            .find(|x| x["ref"] == r)
            .map(|x| x["title"].as_str().unwrap_or("").to_lowercase())
            .unwrap_or_default();
        if title.contains("xtraos") {
            eprintln!("{app}: title = {title}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{app}: search did not load (title: {title})"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    core.ok("close_window", json!({"window": r})).await;
}

// P1-WIN-06 / flow F3: Chrome search → close → Firefox (no extension) search → close.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opt-in: opens the user's Chrome and Firefox"]
async fn win05_flow_f3_chrome_then_firefox() {
    if std::env::var("XTRAOS_TEST_BROWSERS").as_deref() != Ok("1") {
        eprintln!("SKIP: set XTRAOS_TEST_BROWSERS=1 to run (opens new Chrome/Firefox windows)");
        return;
    }
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    browser_search(
        &mut core,
        "chrome",
        json!(["--new-window", "about:blank"]),
        "xtraos agent test",
    )
    .await;
    browser_search(
        &mut core,
        "firefox",
        json!(["-new-window", "about:blank"]),
        "xtraos agent test",
    )
    .await;
    core.stop().await;
}

// P6-M1a / P6-M2 (ADR 0046): `window` by text, the ambiguous case, and the focus result.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens two testbed windows, focuses and closes them"]
async fn win06_window_by_text_and_focus_truth() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let launch =
        json!({"app": testbed_exe(), "args": ["300", "200"], "timeout_ms": 15000, "reuse": false});
    let a = core.ok("launch_app", launch.clone()).await;
    let _ca = EndTestbedOnDrop(a["pid"].as_u64().unwrap());
    let ra = a["ref"].as_str().unwrap().to_string();

    // One testbed window: part of the title (any case) and the process name both find it.
    let b1 = core
        .ok("window_bounds", json!({"window": "xtraos test"}))
        .await;
    assert_eq!(b1["ref"], ra, "{b1}");
    let b2 = core
        .ok("window_bounds", json!({"window": "testbed.exe"}))
        .await;
    assert_eq!(b2["ref"], ra, "{b2}");
    let b3 = core.ok("window_bounds", json!({"window": "testbed"})).await;
    assert_eq!(b3["ref"], ra, "process name without .exe: {b3}");

    // focus_window by text reports the truth: it is in front 300 ms later.
    let f = core
        .ok("focus_window", json!({"window": "XtraOS Testbed"}))
        .await;
    assert_eq!(f["ref"], ra, "{f}");
    assert_eq!(f["focused"], true, "{f}");

    // A second testbed window: the same text is now ambiguous and nothing is chosen.
    let b = core.ok("launch_app", launch).await;
    let _cb = EndTestbedOnDrop(b["pid"].as_u64().unwrap());
    let rb = b["ref"].as_str().unwrap().to_string();
    assert_ne!(ra, rb);
    let e = core
        .call("window_bounds", json!({"window": "XtraOS Testbed"}))
        .await;
    assert_eq!(code(&e), "INVALID_ARGS", "{e}");
    let msg = e.to_string();
    assert!(
        msg.contains(&ra) && msg.contains(&rb),
        "both candidates are named: {msg}"
    );

    // Unknown text is "not found"; refs keep working.
    let e = core
        .call("window_bounds", json!({"window": "no-such-window-xyz"}))
        .await;
    assert_eq!(code(&e), "WINDOW_NOT_FOUND", "{e}");
    core.ok("close_window", json!({"window": ra})).await;
    core.ok("close_window", json!({"window": rb})).await;
    core.stop().await;
}
