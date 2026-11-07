//! P2-M5 admin helper tests (docs/08 P2-EH-*, ADR 0030).
//!
//! Automated (no human): the helper runs UNELEVATED in test mode (`--test-same-integrity`,
//! which an elevated process refuses), so the channel, refusals, routing, approvals and
//! lifetime are tested without a UAC prompt. The test answers the helper's dialog itself.
//! `#[ignore]` because they open windows; run via tests/phase2/p2m5-gate.ps1 -Automated.
//!
//! Human: P2-EH-F5 (real UAC prompt → admin Command Prompt → ipconfig) runs only with
//! XTRAOS_TEST_UAC=1, from p2m5-gate.ps1 -Interactive.
mod common;

use std::ffi::c_void;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{code, refs_by_name, Core, Testbed};
use serde_json::{json, Value};
use xtraos_core::{helper, safety};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[link(name = "user32")]
extern "system" {
    fn SendMessageW(h: *mut c_void, msg: u32, w: usize, l: isize) -> isize;
    fn PostMessageW(h: *mut c_void, msg: u32, w: usize, l: isize) -> i32;
    fn GetDlgItem(h: *mut c_void, id: i32) -> *mut c_void;
    fn GetWindowTextW(h: *mut c_void, buf: *mut u16, n: i32) -> i32;
    fn FindWindowW(class: *const u16, title: *const u16) -> *mut c_void;
}
#[link(name = "kernel32")]
extern "system" {
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
    fn WaitForSingleObject(h: *mut c_void, ms: u32) -> u32;
    fn CloseHandle(h: *mut c_void) -> i32;
}

fn exe() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_xtraos-core"))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn text(v: &Value) -> String {
    v["text"].as_str().unwrap_or("").to_string()
}

/// True once process `pid` has exited (waits up to `ms`).
fn exited(pid: u64, ms: u32) -> bool {
    let h = unsafe { OpenProcess(0x0010_0000, 0, pid as u32) }; // SYNCHRONIZE
    if h.is_null() {
        return true;
    }
    let r = unsafe { WaitForSingleObject(h, ms) };
    unsafe { CloseHandle(h) };
    r == 0
}

/// The human answers the HELPER's dialog (another process) with button `i` within `wait`.
fn human_answers_helper(i: i32, wait: Duration) -> std::thread::JoinHandle<Option<String>> {
    std::thread::spawn(move || {
        let title = wide("XtraOS admin helper — approval needed");
        let t0 = Instant::now();
        while t0.elapsed() < wait {
            let d = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
            if !d.is_null() {
                std::thread::sleep(Duration::from_millis(300));
                let mut buf = [0u16; 2048];
                let st = unsafe { GetDlgItem(d, safety::PROMPT_TEXT_ID) };
                let n = unsafe { GetWindowTextW(st, buf.as_mut_ptr(), 2048) };
                let b = unsafe { GetDlgItem(d, safety::PROMPT_BUTTON_BASE + i) };
                unsafe { SendMessageW(b, 0x00F5, 0, 0) }; // BM_CLICK
                return Some(String::from_utf16_lossy(&buf[..n.max(0) as usize]));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    })
}

fn resume_from_tray() {
    let ui = safety::ui_window().expect("safety UI");
    unsafe { PostMessageW(ui as _, 0x0111, safety::ID_RESUME, 0) };
    for _ in 0..100 {
        if !safety::stopped() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn helper_pid(core_status: &Value) -> u64 {
    core_status["admin_helper"]["pid"].as_u64().unwrap_or(0)
}

// P2-EH-01 + 04: handshake, status, stop → the helper process ends.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "starts the helper process"]
async fn eh01_04_handshake_status_stop() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let off = core.ok("elevated_helper", json!({"op": "status"})).await;
    assert_eq!(text(&off), "admin helper not running", "{off}");
    let r = helper::start_unelevated_for_tests(&exe()).expect("helper starts");
    eprintln!("EH-01 start: {}", text(&r));
    assert_eq!(r["helper"]["elevated"], false, "test mode is unelevated");
    let st = core.ok("status", json!({})).await;
    let pid = helper_pid(&st);
    assert!(pid > 0 && st["admin_helper"]["running"] == true, "{st}");
    let again = helper::start_unelevated_for_tests(&exe()).unwrap();
    assert_eq!(again["already_running"], true, "no second helper");
    let stop = core.ok("elevated_helper", json!({"op": "stop"})).await;
    assert_eq!(text(&stop), "ok admin helper stopped");
    assert!(
        exited(pid, 5000),
        "helper process ended when the core hung up"
    );
    eprintln!("EH-04 ok: helper {pid} exited after stop");
    core.stop().await;
}

// P2-EH-02: a squatted pipe name → the helper refuses to run (exit 6).
#[test]
#[ignore = "starts the helper process"]
fn eh02_squatted_pipe_name_is_refused() {
    let name = format!(r"\\.\pipe\xtraos-elev-{}", common::rand_hex(16));
    let _squat = tokio::runtime::Runtime::new().unwrap().block_on(async {
        tokio::net::windows::named_pipe::ServerOptions::new()
            .first_pipe_instance(true)
            .create(&name)
            .unwrap()
    });
    let st = std::process::Command::new(exe())
        .args([
            "--elevated-helper",
            "--pipe",
            &name,
            "--core-pid",
            &std::process::id().to_string(),
            "--test-same-integrity",
        ])
        .status()
        .unwrap();
    assert_eq!(st.code(), Some(6), "squatted name → refuse");
    eprintln!("EH-02 ok: squatted pipe refused (exit 6)");
}

// P2-EH-03: only the core's PID may connect; anyone else ends the helper (exit 8). Bad
// arguments are refused (exit 2).
#[test]
#[ignore = "starts the helper process"]
fn eh03_foreign_client_is_refused() {
    let name = format!(r"\\.\pipe\xtraos-elev-{}", common::rand_hex(16));
    // The "core" is some other process; this test process is the intruder.
    let mut other = std::process::Command::new("cmd")
        .args(["/c", "ping -n 30 127.0.0.1 >nul"])
        .spawn()
        .unwrap();
    let mut h = std::process::Command::new(exe())
        .args([
            "--elevated-helper",
            "--pipe",
            &name,
            "--core-pid",
            &other.id().to_string(),
            "--test-same-integrity",
        ])
        .spawn()
        .unwrap();
    let t0 = Instant::now();
    let pipe = loop {
        if let Ok(f) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
        {
            break f;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "pipe never appeared"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    let st = h.wait().unwrap();
    drop(pipe);
    let _ = other.kill();
    let _ = other.wait();
    assert_eq!(st.code(), Some(8), "foreign client → refused");
    let bad = std::process::Command::new(exe())
        .args([
            "--elevated-helper",
            "--pipe",
            r"\\.\pipe\evil",
            "--core-pid",
            "1",
        ])
        .status()
        .unwrap();
    assert_eq!(bad.code(), Some(2), "bad pipe name refused");
    eprintln!("EH-03 ok: foreign client refused (exit 8), bad args (exit 2)");
}

// P2-EH-05 + 06: routing with helper refs, the helper's own approval (Allow window → no more
// dialogs; Deny → blocked), the dialog shows the text, refs without a helper are STALE_REF.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "opens the testbed and the helper"]
async fn eh05_06_routing_and_helper_approval() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(160, 160);
    let stale = core.call("click", json!({"ref": "e50000003"})).await;
    assert_eq!(
        code(&stale),
        "STALE_REF",
        "helper refs need the helper: {stale}"
    );

    helper::start_unelevated_for_tests(&exe()).expect("helper");
    // The testbed is not elevated, so force this observe through the helper (test hook =
    // the helper API itself); in production the core forwards on ELEVATION_REQUIRED.
    let human = human_answers_helper(1, Duration::from_secs(15)); // Allow this window 10 min
    let o =
        helper::forward("observe", &json!({"window": tb.window()})).expect("observe via helper");
    let msg = human
        .join()
        .unwrap()
        .expect("helper dialog for the first action");
    eprintln!("helper dialog:\n{msg}");
    assert!(msg.contains("ADMINISTRATOR window (testbed.exe)"), "{msg}");
    assert_eq!(o["via"], "admin_helper");
    let refs = refs_by_name(&text(&o));
    let btn = refs["btnA"].clone();
    let n: u32 = btn[1..].parse().unwrap();
    assert!(
        n >= 50_000_000,
        "helper refs are in the helper range: {btn}"
    );

    // Through the core pipe: helper ref → forwarded; window granted → no dialog.
    let watcher = human_answers_helper(2, Duration::from_secs(3));
    let r = core.call("click", json!({"ref": btn})).await;
    assert!(
        watcher.join().unwrap().is_none(),
        "granted window: no second dialog"
    );
    assert!(r.get("result").is_some(), "{r}");
    assert_eq!(
        tb.state()["clicks"]["btnA"],
        1,
        "the click reached the window"
    );
    eprintln!("EH-05 ok: {btn} clicked via the helper, one dialog");

    // A second window is not granted: Deny blocks, and the dialog shows the text.
    let mut tb2 = Testbed::start(700, 160);
    let human = human_answers_helper(2, Duration::from_secs(15)); // Deny
    let d = helper::forward(
        "set_value",
        &json!({"window": tb2.window(), "value": "rm -rf"}),
    );
    let msg = human
        .join()
        .unwrap()
        .expect("dialog for an ungranted window");
    assert!(
        msg.contains("Value: \"rm -rf\""),
        "dialog shows what is typed: {msg}"
    );
    assert_eq!(d.unwrap_err().code, "BLOCKED_BY_POLICY", "denied");
    let _ = tb2.state();
    eprintln!("EH-06 ok: Deny blocks; the dialog shows the value");
    let _ = core.ok("elevated_helper", json!({"op": "stop"})).await;
    let _ = tb.state();
    core.stop().await;
}

// P2-EH-07: the kill switch ends the helper; after Resume it is not running (new UAC needed).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "starts the helper process"]
async fn eh07_kill_switch_ends_the_helper() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    helper::start_unelevated_for_tests(&exe()).expect("helper");
    let st = core.ok("status", json!({})).await;
    let pid = helper_pid(&st);
    assert!(pid > 0);
    safety::stop();
    assert!(exited(pid, 5000), "the kill switch ended the helper");
    resume_from_tray();
    let s = core.ok("elevated_helper", json!({"op": "status"})).await;
    assert_eq!(text(&s), "admin helper not running", "{s}");
    eprintln!("EH-07 ok: kill switch ended helper {pid}");
    core.stop().await;
}

// P2-EH-F5 (human): real UAC prompt, admin Command Prompt, `ipconfig` through the helper.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "human: approve the UAC prompt and the helper dialog"]
async fn eh_f5_admin_command_prompt() {
    if std::env::var("XTRAOS_TEST_UAC").ok().as_deref() != Some("1") {
        eprintln!("SKIP: set XTRAOS_TEST_UAC=1 and approve the prompts yourself");
        return;
    }
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let r = core.call("elevated_helper", json!({"op": "start"})).await;
    eprintln!("F5 helper: {r}");
    assert!(r.get("result").is_some(), "{r}");
    let w = core
        .ok(
            "launch_app",
            json!({"app": "cmd", "elevated": true, "timeout_ms": 60000}),
        )
        .await;
    let win = w["ref"].as_str().unwrap().to_string();
    let t = core
        .ok(
            "type",
            json!({"window": win, "text": "ipconfig", "submit": true}),
        )
        .await;
    eprintln!("F5 type: {t}");
    assert_eq!(t["via"], "admin_helper");
    std::thread::sleep(Duration::from_millis(1500));
    let out = core.ok("read_text", json!({"window": win})).await;
    assert!(
        text(&out).contains("IPv4") || text(&out).contains("Windows IP"),
        "{out}"
    );
    let _ = core.call("close_window", json!({"window": win})).await;
    let _ = core.ok("elevated_helper", json!({"op": "stop"})).await;
    core.stop().await;
}
