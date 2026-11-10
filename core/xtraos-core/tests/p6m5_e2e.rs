//! P6-M5 page content of Chromium windows (ADR 0048), interactive and opt-in
//! (XTRAOS_TEST_BROWSERS=1): Edge and Chrome with throw-away profiles in private mode.
//! - the first `observe` of a just-opened page already has the page (it waits for it);
//! - a window minimized before its first read says `page: "minimized"`, `reveal: true` reads
//!   the page, the window ends minimized and the foreground window does not change.
mod common;

use std::ffi::c_void;
use std::time::{Duration, Instant};

use common::Core;
use serde_json::{json, Value};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[link(name = "user32")]
extern "system" {
    fn GetForegroundWindow() -> *mut c_void;
}

/// Ends only this test's browser (its profile folder carries the tag) and deletes the folder.
struct EndBrowserOnDrop(String, &'static str);

impl Drop for EndBrowserOnDrop {
    fn drop(&mut self) {
        let ps = format!(
            "Get-CimInstance Win32_Process -Filter \"Name='{1}'\" | Where-Object {{ $_.CommandLine -like '*xtraos-p6m5-{0}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
            self.0, self.1
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .status();
        std::thread::sleep(Duration::from_secs(1));
        let _ = std::fs::remove_dir_all(std::env::temp_dir().join(format!("xtraos-p6m5-{}", self.0)));
    }
}

struct Browser {
    exe: std::path::PathBuf,
    image: &'static str,
    private: &'static str,
}

fn browsers() -> Vec<Browser> {
    let mut v = Vec::new();
    let edge = std::path::Path::new(&std::env::var("ProgramFiles(x86)").unwrap_or_default())
        .join(r"Microsoft\Edge\Application\msedge.exe");
    if edge.exists() {
        v.push(Browser { exe: edge, image: "msedge.exe", private: "--inprivate" });
    }
    let chrome = std::path::Path::new(&std::env::var("ProgramFiles").unwrap_or_default())
        .join(r"Google\Chrome\Application\chrome.exe");
    if chrome.exists() {
        v.push(Browser { exe: chrome, image: "chrome.exe", private: "--incognito" });
    }
    v
}

/// Opens the test page in a new private browser process; returns its window ref (found by
/// title only: no UI Automation request before the test's first observe).
async fn open(core: &mut Core, b: &Browser) -> (String, EndBrowserOnDrop) {
    let tag = common::rand_hex(6);
    let guard = EndBrowserOnDrop(tag.clone(), b.image);
    let dir = std::env::temp_dir().join(format!("xtraos-p6m5-{tag}"));
    std::fs::create_dir_all(&dir).unwrap();
    let title = format!("XtraOS page test {tag}");
    let mut body = String::new();
    for i in 1..=60 {
        body.push_str(&format!("<a href='#{i}'>link {i}</a> "));
    }
    for i in 1..=20 {
        body.push_str(&format!("<button>btn {i}</button>"));
    }
    let page = dir.join("page.html");
    std::fs::write(&page, format!("<title>{title}</title>{body}")).unwrap();
    let url = format!("file:///{}", page.display().to_string().replace('\\', "/"));
    std::process::Command::new(&b.exe)
        .args([
            &format!("--user-data-dir={}", dir.join("profile").display()),
            b.private,
            "--no-first-run",
            "--new-window",
            "--window-position=120,120",
            "--window-size=900,700",
            &url,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let t0 = Instant::now();
    loop {
        let r = core.call("window_bounds", json!({"window": title})).await;
        if let Some(w) = r["result"]["ref"].as_str() {
            return (w.to_string(), guard);
        }
        assert!(t0.elapsed() < Duration::from_secs(20), "{} window: {r}", b.image);
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

fn has_page(r: &Value) -> bool {
    let t = r["text"].as_str().unwrap_or("");
    t.contains("\"link 1\"") && t.contains("\"btn 20\"")
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive + browsers: opens Edge/Chrome with throw-away profiles"]
async fn p6m5_first_read_and_reveal() {
    if std::env::var("XTRAOS_TEST_BROWSERS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set XTRAOS_TEST_BROWSERS=1");
        return;
    }
    let _s = SERIAL.lock().await;
    let list = browsers();
    assert!(!list.is_empty(), "no Edge or Chrome installed");
    let mut core = Core::start().await;
    for b in &list {
        // 1. The very first observe of a just-opened page has the page.
        let (w, _g) = open(&mut core, b).await;
        let r = core.ok("observe", json!({"window": w, "full": true})).await;
        assert!(has_page(&r), "{}: first read has the page: {r}", b.image);
        assert!(r["page"].is_null(), "{r}");
        eprintln!("{}: first read waited {} ms", b.image, r["waited_ms"]);
        core.ok("close_window", json!({"window": w})).await;

        // 2. Minimized before its first read: says why; reveal reads it without focus change.
        let (w, _g2) = open(&mut core, b).await;
        core.ok("window_bounds", json!({"window": w, "state": "minimize"})).await;
        tokio::time::sleep(Duration::from_secs(2)).await;
        let r = core.ok("observe", json!({"window": w, "full": true})).await;
        assert_eq!(r["page"], "minimized", "{}: {r}", b.image);
        assert!(r["text"].as_str().unwrap().contains("reveal:true"), "{r}");
        let fg = unsafe { GetForegroundWindow() } as usize;
        let r = core
            .ok("observe", json!({"window": w, "full": true, "reveal": true}))
            .await;
        assert!(has_page(&r), "{}: revealed page: {r}", b.image);
        assert_eq!(r["revealed"], true, "{r}");
        assert!(r["page"].is_null(), "{r}");
        assert_eq!(unsafe { GetForegroundWindow() } as usize, fg, "focus unchanged");
        let st = core.ok("window_bounds", json!({"window": w})).await;
        assert_eq!(st["state"], "minimized", "minimized again: {st}");
        // The page stays readable while minimized.
        let r = core.ok("observe", json!({"window": w, "full": true})).await;
        assert!(has_page(&r), "{}: still readable: {r}", b.image);
        core.ok("close_window", json!({"window": w})).await;
    }
    core.stop().await;
}
