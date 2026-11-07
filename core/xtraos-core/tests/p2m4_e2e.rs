//! P2-M4 interactive tests (docs/08 P2-RES-*, P2-WA-*): resource discipline and the
//! whatsapp.send skill. Private XtraOS data folder (never the user's real app map / policy).
//! `#[ignore]`: run via tests/phase2/p2m4-gate.ps1 -Interactive, hands off.
//!
//! Real WhatsApp sends (P2-WA-01/02) run only when `XTRAOS_TEST_WHATSAPP` holds the OWNER'S OWN
//! number (self chat). The number is never stored in the repo. Without it those tests skip.
mod common;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use common::{code, testbed_exe, Core, Testbed};
use serde_json::{json, Value};
use xtraos_core::safety;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[link(name = "user32")]
extern "system" {
    fn GetDlgItem(h: *mut std::ffi::c_void, id: i32) -> *mut std::ffi::c_void;
    fn GetWindowTextW(h: *mut std::ffi::c_void, buf: *mut u16, n: i32) -> i32;
    fn IsWindow(h: *mut std::ffi::c_void) -> i32;
}

fn text(v: &Value) -> String {
    v["result"]["text"]
        .as_str()
        .or_else(|| v["text"].as_str())
        .unwrap_or("")
        .to_string()
}

fn private_home() -> PathBuf {
    let home = std::env::temp_dir().join(format!("xtraos-p2m4-home-{}", std::process::id()));
    std::fs::create_dir_all(home.join("skills")).unwrap();
    std::env::set_var("XTRAOS_HOME", &home);
    xtraos_core::appmap::set_path(home.join("appmap.bin"));
    safety::set_policy_path(home.join("policy.json"));
    home
}

/// The human presses button `i` (0 = Allow once, 1 = Deny) in the next core dialog within
/// `wait`; returns its message, or None when no dialog came.
fn human_answers(i: i32, wait: Duration) -> std::thread::JoinHandle<Option<String>> {
    std::thread::spawn(move || {
        let t0 = Instant::now();
        while t0.elapsed() < wait {
            if let Some(&d) = safety::prompt_windows().first() {
                std::thread::sleep(Duration::from_millis(300));
                let mut buf = [0u16; 2048];
                let st = unsafe { GetDlgItem(d as _, safety::PROMPT_TEXT_ID) };
                let n = unsafe { GetWindowTextW(st, buf.as_mut_ptr(), 2048) };
                xtraos_core::safety::answer_prompt_for_tests(i as usize);
                return Some(String::from_utf16_lossy(&buf[..n.max(0) as usize]));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    })
}

fn alive(r: &str) -> bool {
    let h: usize = r[1..].parse().unwrap();
    unsafe { IsWindow(h as _) != 0 }
}

async fn launch_tb(core: &mut Core, x: i32, reuse: bool) -> Value {
    core.ok(
        "launch_app",
        json!({"app": testbed_exe().to_string_lossy(), "args": [x.to_string(), "200"],
               "reuse": reuse, "timeout_ms": 20000}),
    )
    .await
}

fn agent_flag(list: &Value, r: &str) -> Option<bool> {
    list["windows"]
        .as_array()?
        .iter()
        .find(|w| w["ref"] == r)
        .map(|w| w["agent"].as_bool().unwrap_or(false))
}

// P2-RES-01 + 02 + 03: reuse instead of a duplicate; `agent` marks; cleanup closes only
// agent windows (the user's testbed stays open).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens testbed windows"]
async fn res01_03_reuse_marks_and_cleanup() {
    let _s = SERIAL.lock().await;
    let home = private_home();
    let user_tb = Testbed::start(700, 160); // "the user's" window: not opened by the agent
    let mut core = Core::start().await;
    let a = launch_tb(&mut core, 140, true).await;
    let ra = a["ref"].as_str().unwrap().to_string();
    assert!(a.get("reused").is_none(), "first launch is new: {a}");
    let again = launch_tb(&mut core, 140, true).await;
    assert_eq!(again["reused"], true, "same app + args → reused: {again}");
    assert_eq!(again["ref"], ra, "same window, no duplicate");
    let b = launch_tb(&mut core, 140, false).await;
    let rb = b["ref"].as_str().unwrap().to_string();
    assert_ne!(rb, ra, "reuse=false opens a new window");
    eprintln!("RES-01 ok: {ra} reused, {rb} new");

    let list = core.ok("list_windows", json!({"filter": "testbed"})).await;
    assert_eq!(agent_flag(&list, &ra), Some(true), "{list}");
    assert_eq!(agent_flag(&list, &rb), Some(true), "{list}");
    assert_eq!(agent_flag(&list, &user_tb.window()), Some(false), "{list}");
    eprintln!("RES-02 ok: agent marks");

    // cleanup is class `close` (allowed): no dialog expected.
    let watcher = human_answers(1, Duration::from_secs(4));
    let c = core.call("cleanup", json!({})).await;
    eprintln!("cleanup: {}", text(&c));
    assert!(watcher.join().unwrap().is_none(), "cleanup must not ask");
    assert_eq!(c["result"]["closed"], 2, "{c}");
    std::thread::sleep(Duration::from_millis(500));
    assert!(!alive(&ra) && !alive(&rb), "agent windows closed");
    assert!(alive(&user_tb.window()), "the user's window is untouched");
    let c2 = core.ok("cleanup", json!({})).await;
    assert_eq!(c2["closed"], 0, "nothing left: {c2}");
    eprintln!("RES-03 ok");
    core.stop().await;
    drop(user_tb);
    let _ = std::fs::remove_dir_all(home);
}

// P2-RES-04: closing a window the agent did not open asks (deny → stays open); closing an
// agent window does not ask.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn res04_closing_a_user_window_asks() {
    let _s = SERIAL.lock().await;
    let home = private_home();
    let user_tb = Testbed::start(700, 160);
    let mut core = Core::start().await;
    let human = human_answers(1, Duration::from_secs(10)); // Deny
    let r = core
        .call(
            "close_window",
            json!({"window": user_tb.window(), "timeout_ms": 3000}),
        )
        .await;
    let msg = human
        .join()
        .unwrap()
        .expect("a dialog for the user's window");
    eprintln!("close dialog:\n{msg}");
    assert!(msg.contains("a window you opened"), "{msg}");
    assert_eq!(code(&r), "BLOCKED_BY_POLICY", "{r}");
    assert!(alive(&user_tb.window()), "denied: still open");

    let a = launch_tb(&mut core, 140, true).await;
    let ra = a["ref"].as_str().unwrap().to_string();
    let watcher = human_answers(1, Duration::from_secs(4));
    let r = core
        .call("close_window", json!({"window": ra, "timeout_ms": 3000}))
        .await;
    assert!(
        watcher.join().unwrap().is_none(),
        "agent window closes without asking"
    );
    assert!(r.get("result").is_some(), "{r}");
    eprintln!("RES-04 ok");
    core.stop().await;
    drop(user_tb);
    let _ = std::fs::remove_dir_all(home);
}

fn owner_number() -> Option<String> {
    std::env::var("XTRAOS_TEST_WHATSAPP")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

async fn map_whatsapp(core: &mut Core) {
    core.ok(
        "app_map",
        json!({"op": "set", "service": "whatsapp", "surface": "browser",
               "location": {"browser": "firefox", "url": "https://web.whatsapp.com"}}),
    )
    .await;
}

async fn wa(core: &mut Core, args: Value) -> Value {
    core.call("run_skill", json!({"name": "whatsapp.send", "args": args}))
        .await
}

// P2-WA-03 + 04: deny → nothing happens; a bad number is refused before any dialog; without
// an app-map entry the approved skill stops before touching anything. No message is sent by
// this test (dummy number, denied or stopped before opening WhatsApp).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn wa03_04_deny_bad_number_no_map() {
    let _s = SERIAL.lock().await;
    let home = private_home();
    let mut core = Core::start().await;
    let before = xtraos_core::winmgr::listable_windows().len();

    let r = wa(&mut core, json!({"phone": "12-34", "text": "x"})).await;
    assert_eq!(code(&r), "INVALID_ARGS", "{r}");
    assert!(
        safety::prompt_windows().is_empty(),
        "no dialog for bad input"
    );

    let human = human_answers(1, Duration::from_secs(10)); // Deny
    let r = wa(
        &mut core,
        json!({"phone": "+00 00000 00000", "text": "XtraOS deny test"}),
    )
    .await;
    let msg = human.join().unwrap().expect("a send dialog");
    eprintln!("send dialog:\n{msg}");
    assert!(
        msg.contains("+000000000000") && msg.contains("XtraOS deny test"),
        "{msg}"
    );
    assert_eq!(code(&r), "BLOCKED_BY_POLICY", "{r}");

    let human = human_answers(0, Duration::from_secs(10)); // Allow once
    let r = wa(&mut core, json!({"phone": "+00 00000 00000", "text": "x"})).await;
    human.join().unwrap().expect("dialog");
    assert_eq!(code(&r), "ELEMENT_NOT_FOUND", "no app map entry: {r}");
    assert_eq!(
        xtraos_core::winmgr::listable_windows().len(),
        before,
        "nothing opened"
    );
    eprintln!("WA-03/04 ok");
    core.stop().await;
    let _ = std::fs::remove_dir_all(home);
}

// P2-WA-01: send a text to the owner's own number; the skill closes what it opened.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: sends a WhatsApp message to XTRAOS_TEST_WHATSAPP"]
async fn wa01_send_text_to_owner() {
    let Some(phone) = owner_number() else {
        eprintln!("SKIP: set XTRAOS_TEST_WHATSAPP to your own number");
        return;
    };
    let _s = SERIAL.lock().await;
    let home = private_home();
    let mut core = Core::start().await;
    map_whatsapp(&mut core).await;
    let msg = format!("XtraOS Agent test {}", common::rand_hex(2));
    let human = human_answers(0, Duration::from_secs(10));
    let t0 = Instant::now();
    let r = wa(&mut core, json!({"phone": phone, "text": msg})).await;
    let dialog = human.join().unwrap().expect("a send dialog");
    assert!(dialog.contains(&msg), "dialog shows the message: {dialog}");
    eprintln!("WA-01: {} in {:?}", text(&r), t0.elapsed());
    assert!(r.get("result").is_some(), "{r}");
    assert!(text(&r).starts_with("ok sent"), "{r}");
    assert!(!text(&r).contains(&msg), "result never echoes chat content");
    if r["result"]["window_reused"] == false {
        assert_eq!(r["result"]["window_closed"], true, "{r}");
        assert!(
            xtraos_core::winmgr::agent_open_windows().is_empty(),
            "the skill closed the window it opened"
        );
    }
    core.stop().await;
    let _ = std::fs::remove_dir_all(home);
}

// P2-WA-02: send a small file (with a caption) to the owner's own number; the clipboard text
// is restored.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: sends a WhatsApp file to XTRAOS_TEST_WHATSAPP"]
async fn wa02_send_file_to_owner() {
    let Some(phone) = owner_number() else {
        eprintln!("SKIP: set XTRAOS_TEST_WHATSAPP to your own number");
        return;
    };
    let _s = SERIAL.lock().await;
    let home = private_home();
    let mut core = Core::start().await;
    map_whatsapp(&mut core).await;
    let dir = std::env::temp_dir().join(format!("xtraos-wa-{}", common::rand_hex(3)));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("xtraos-file-test.txt");
    std::fs::write(&f, "XtraOS Agent file test\n").unwrap();
    let marker = format!("clip-{}", common::rand_hex(4));
    core.ok("clipboard", json!({"op": "set", "text": marker}))
        .await;
    let human = human_answers(0, Duration::from_secs(10));
    let r = wa(
        &mut core,
        json!({"phone": phone, "file": f.to_string_lossy(), "text": "XtraOS file test"}),
    )
    .await;
    let dialog = human.join().unwrap().expect("a send dialog");
    assert!(
        dialog.contains("xtraos-file-test.txt"),
        "dialog names the file: {dialog}"
    );
    eprintln!("WA-02: {}", text(&r));
    assert!(text(&r).starts_with("ok sent file"), "{r}");
    let clip = core.ok("clipboard", json!({"op": "get"})).await;
    assert_eq!(
        clip["text"], marker,
        "the user's clipboard text is restored"
    );
    core.stop().await;
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(home);
}
