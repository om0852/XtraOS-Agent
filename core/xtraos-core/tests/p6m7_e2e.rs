//! P6-M7 small tools (ADR 0051), interactive: a plain wait, a notification (shows a Windows
//! notification once), and launch_app with an internal browser page (opt-in browsers:
//! XTRAOS_TEST_BROWSERS=1, Chrome incognito with a throw-away profile; hands off — the address
//! bar typing may borrow the window).
mod common;

use std::time::{Duration, Instant};

use common::Core;
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: shows one Windows notification"]
async fn p6m7_wait_and_notify() {
    let mut core = Core::start().await;
    let t0 = Instant::now();
    let r = core.ok("wait_for", json!({"delay_ms": 1500})).await;
    assert!(t0.elapsed() >= Duration::from_millis(1500), "{r}");
    assert_eq!(r["met"], json!(["delay"]), "{r}");
    let e = core.call("wait_for", json!({})).await;
    assert_eq!(common::code(&e), "INVALID_ARGS", "{e}");
    let e = core.call("wait_for", json!({"delay_ms": 70000})).await;
    assert_eq!(common::code(&e), "INVALID_ARGS", "{e}");

    let n = core
        .ok("ask_human", json!({"kind": "notify", "message": "XtraOS test: notifications work"}))
        .await;
    assert_eq!(n["shown"], true, "{n}");
    let n2 = core
        .ok("ask_human", json!({"kind": "notify", "message": "second"}))
        .await;
    assert_eq!(n2["throttled"], true, "{n2}");
    core.stop().await;
}

struct EndChromeOnDrop(String);

impl Drop for EndChromeOnDrop {
    fn drop(&mut self) {
        let ps = format!(
            "Get-CimInstance Win32_Process -Filter \"Name='chrome.exe'\" | Where-Object {{ $_.CommandLine -like '*xtraos-p6m7-{0}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
            self.0
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .status();
        std::thread::sleep(Duration::from_secs(1));
        let dir = std::env::temp_dir().join(format!("xtraos-p6m7-{}", self.0));
        let _ = std::process::Command::new("cmd")
            .args(["/c", "rd", "/s", "/q"])
            .arg(&dir)
            .status();
    }
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive + browsers: opens Chrome incognito with a throw-away profile"]
async fn p6m7_launch_app_opens_an_internal_page() {
    if std::env::var("XTRAOS_TEST_BROWSERS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set XTRAOS_TEST_BROWSERS=1");
        return;
    }
    let chrome = std::path::Path::new(&std::env::var("ProgramFiles").unwrap_or_default())
        .join(r"Google\Chrome\Application\chrome.exe");
    if !chrome.exists() {
        eprintln!("skipped: Chrome not installed");
        return;
    }
    let tag = common::rand_hex(6);
    let _end = EndChromeOnDrop(tag.clone());
    let dir = std::env::temp_dir().join(format!("xtraos-p6m7-{tag}"));
    let mut core = Core::start().await;
    let r = core
        .ok(
            "launch_app",
            json!({
                "app": chrome.display().to_string(),
                "args": [format!("--user-data-dir={}", dir.display()), "--incognito", "--no-first-run", "--new-window", "chrome://version"],
                "timeout_ms": 20000,
                "reuse": false,
            }),
        )
        .await;
    assert_eq!(r["opened_internal"], "chrome://version", "{r}");
    let t0 = Instant::now();
    let mut title = String::new();
    // The loaded page is titled "About Version"; while loading the title is the typed address.
    while t0.elapsed() < Duration::from_secs(8) {
        let b = core.ok("list_windows", json!({"filter": "About Version"})).await;
        if let Some(t) = b["windows"][0]["title"].as_str() {
            title = t.to_string();
            break;
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(title.contains("About Version"), "the internal page is open: {title:?} / {r}");
    core.stop().await;
}
