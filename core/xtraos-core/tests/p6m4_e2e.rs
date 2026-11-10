//! P6-M4 dialog awareness (ADR 0047), interactive: a real owned message box in the testbed and
//! (opt-in, XTRAOS_TEST_BROWSERS=1) a JavaScript alert in Edge with a throw-away profile.
mod common;

use std::time::{Duration, Instant};

use common::{Core, Testbed};
use serde_json::{json, Value};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// observe until `pred(result)` holds (dialogs appear and close asynchronously).
async fn observe_until(core: &mut Core, window: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let t0 = Instant::now();
    loop {
        let r = core
            .ok("observe", json!({"window": window, "full": true}))
            .await;
        if pred(&r) || t0.elapsed() > Duration::from_secs(8) {
            return r;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn button(r: &Value, name: &str) -> String {
    r["dialog"]["buttons"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == name)
        .unwrap_or_else(|| panic!("no '{name}' button: {r}"))["ref"]
        .as_str()
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens the testbed with a modal message box"]
async fn p6m4_owned_message_box_is_reported_first() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(300, 200);
    let w = tb.window();

    let r = observe_until(&mut core, &w, |r| r["dialog"].is_null()).await;
    assert!(r["dialog"].is_null(), "no dialog yet: {r}");

    tb.cmd("modal");
    let r = observe_until(&mut core, &w, |r| !r["dialog"].is_null()).await;
    let d = &r["dialog"];
    assert_eq!(d["kind"], "window", "{r}");
    assert_eq!(d["blocking"], true, "{r}");
    assert_eq!(d["title"], "XtraOS Confirm", "{r}");
    assert!(d["text"].as_str().unwrap().contains("Save changes?"), "{r}");
    let text = r["text"].as_str().unwrap();
    let second = text.lines().nth(1).unwrap();
    assert!(
        second.starts_with("!! dialog w") && second.contains("(blocking)") && second.contains("\"No\""),
        "the dialog is the line after the header: {text}"
    );
    let dref = d["ref"].as_str().unwrap().to_string();

    // observe of the dialog window itself names its owner.
    let o = core.ok("observe", json!({"window": dref, "full": true})).await;
    assert!(
        o["text"].as_str().unwrap().lines().next().unwrap().contains(&format!("dialog of {w}")),
        "{o}"
    );

    // status names it once it is in front.
    // Windows may refuse the foreground (P6-M2 reports it): retry until it is really in front.
    let mut s = Value::Null;
    for _ in 0..4 {
        let f = core.ok("focus_window", json!({"window": dref})).await;
        s = core.ok("status", json!({})).await;
        if !s["dialog"].is_null() {
            break;
        }
        eprintln!("focus_window: {f}; status.dialog null, retrying");
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    assert_eq!(s["dialog"]["window"], dref, "{s}");
    assert_eq!(s["dialog"]["owner"], w, "{s}");
    assert_eq!(s["dialog"]["blocking"], true, "{s}");

    // Its button refs work like any other ref; the app gets the answer.
    let no = button(&r, "No");
    core.ok("click", json!({"ref": no})).await;
    let t0 = Instant::now();
    while tb.state()["modal"] != "No" && t0.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(tb.state()["modal"], "No");
    let r = observe_until(&mut core, &w, |r| r["dialog"].is_null()).await;
    assert!(r["dialog"].is_null(), "dialog gone: {r}");
    let s = core.ok("status", json!({})).await;
    assert!(s["dialog"].is_null() || s["dialog"]["owner"] != w, "{s}");
    core.stop().await;
}

/// Ends only this test's Edge (its profile folder carries the tag) and deletes the folder, even
/// when an assertion fails: Edge inherits the runner's pipes and would keep cargo waiting.
struct EndEdgeOnDrop(String);

impl Drop for EndEdgeOnDrop {
    fn drop(&mut self) {
        let ps = format!(
            "Get-CimInstance Win32_Process -Filter \"Name='msedge.exe'\" | Where-Object {{ $_.CommandLine -like '*xtraos-p6m4-{0}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
            self.0
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .status();
        std::thread::sleep(Duration::from_secs(1));
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("xtraos-p6m4-{}", self.0)));
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive + browser: opens Edge with a throw-away profile and a JS alert"]
async fn p6m4_browser_alert_is_reported_first() {
    if std::env::var("XTRAOS_TEST_BROWSERS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set XTRAOS_TEST_BROWSERS=1");
        return;
    }
    let _s = SERIAL.lock().await;
    let tag = common::rand_hex(6);
    let _end = EndEdgeOnDrop(tag.clone());
    let dir = std::env::temp_dir().join(format!("xtraos-p6m4-{tag}"));
    std::fs::create_dir_all(&dir).unwrap();
    let page = dir.join("alert.html");
    let title = format!("XtraOS dialog test {tag}");
    std::fs::write(
        &page,
        format!("<title>{title}</title><h1>dialog test</h1><script>setTimeout(()=>alert('XtraOS test alert'),1500)</script>"),
    )
    .unwrap();
    let edge = std::path::Path::new(&std::env::var("ProgramFiles(x86)").unwrap())
        .join(r"Microsoft\Edge\Application\msedge.exe");
    let url = format!("file:///{}", page.display().to_string().replace('\\', "/"));
    std::process::Command::new(edge)
        .args([
            &format!("--user-data-dir={}", dir.join("profile").display()),
            // InPrivate: never signs in to or syncs the user's account (a plain fresh profile
            // auto-signed in and restored synced tabs).
            "--inprivate",
            "--no-first-run",
            "--new-window",
            &url,
        ])
        // Edge outlives this call; it must not hold the test runner's output pipes open.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();

    let mut core = Core::start().await;
    // The window by its title (P6-M1a).
    let t0 = Instant::now();
    let w = loop {
        let b = core
            .call("window_bounds", json!({"window": title}))
            .await;
        if let Some(r) = b["result"]["ref"].as_str() {
            break r.to_string();
        }
        assert!(t0.elapsed() < Duration::from_secs(20), "Edge window: {b}");
        tokio::time::sleep(Duration::from_millis(300)).await;
    };
    let r = observe_until(&mut core, &w, |r| !r["dialog"].is_null()).await;
    let d = &r["dialog"];
    assert_eq!(d["kind"], "in_page", "{r}");
    assert!(d["text"].as_str().unwrap().contains("XtraOS test alert"), "{r}");
    let second = r["text"].as_str().unwrap().lines().nth(1).unwrap().to_string();
    assert!(second.starts_with("!! dialog e"), "{second}");
    let ok = button(&r, "OK");
    core.ok("click", json!({"ref": ok})).await;
    let r = observe_until(&mut core, &w, |r| r["dialog"].is_null()).await;
    assert!(r["dialog"].is_null(), "alert answered: {r}");
    core.ok("close_window", json!({"window": w})).await;
    core.stop().await;
}
