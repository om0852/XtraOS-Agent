//! ADR 0041 security fixes, end to end. SEC-01/02 send no real input (messages only);
//! SEC-03 injects one SendInput click on XtraOS's own dialog and runs in the hands-off gate.
mod common;

use std::ffi::c_void;
use std::time::{Duration, Instant};

use common::{code, Core};
use serde_json::json;
use xtraos_core::safety;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[link(name = "user32")]
extern "system" {
    fn SendMessageW(h: *mut c_void, msg: u32, w: usize, l: isize) -> isize;
    fn PostMessageW(h: *mut c_void, msg: u32, w: usize, l: isize) -> i32;
    fn GetDlgItem(h: *mut c_void, id: i32) -> *mut c_void;
    fn IsWindow(h: *mut c_void) -> i32;
}
const BM_CLICK: u32 = 0x00F5;
const WM_COMMAND: u32 = 0x0111;

fn policy_confirming_observe() {
    let home = std::env::temp_dir().join(format!("xtraos-sec-{}", common::rand_hex(3)));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(
        home.join("policy.json"),
        r#"{"classes": {"observe": "confirm"}}"#,
    )
    .unwrap();
    safety::set_policy_path(home.join("policy.json"));
}

fn wait_dialog() -> usize {
    let t0 = Instant::now();
    loop {
        if let Some(&d) = safety::prompt_windows().first() {
            std::thread::sleep(Duration::from_millis(300));
            return d;
        }
        assert!(t0.elapsed() < Duration::from_secs(10), "no dialog");
        std::thread::sleep(Duration::from_millis(20));
    }
}

// SEC-01: another program "clicking" Allow (BM_CLICK) is ignored; Deny is accepted.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "shows a core dialog"]
async fn sec01_spoofed_allow_is_ignored() {
    let _s = SERIAL.lock().await;
    policy_confirming_observe();
    let mut core = Core::start().await;
    let spoof = std::thread::spawn(|| {
        let d = wait_dialog();
        let allow = unsafe { GetDlgItem(d as _, safety::PROMPT_BUTTON_BASE) };
        unsafe { SendMessageW(allow, BM_CLICK, 0, 0) };
        std::thread::sleep(Duration::from_millis(500));
        let still_open = unsafe { IsWindow(d as _) } != 0;
        let deny = unsafe { GetDlgItem(d as _, safety::PROMPT_BUTTON_BASE + 1) };
        unsafe { SendMessageW(deny, BM_CLICK, 0, 0) };
        still_open
    });
    let r = core.call("list_windows", json!({})).await;
    let still_open = spoof.join().unwrap();
    eprintln!(
        "SEC-01: dialog still open after a spoofed Allow: {still_open}; result {}",
        code(&r)
    );
    assert!(still_open, "a BM_CLICK on Allow must not approve");
    assert_eq!(
        code(&r),
        "BLOCKED_BY_POLICY",
        "Deny (the safe answer) is accepted: {r}"
    );
    core.stop().await;
}

// SEC-02: a posted WM_COMMAND "Resume" no longer undoes the kill switch; Stop still works.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "flips the kill switch"]
async fn sec02_posted_resume_is_ignored() {
    let _s = SERIAL.lock().await;
    let core = Core::start().await;
    let ui = safety::ui_window().expect("safety UI");
    safety::stop();
    unsafe { PostMessageW(ui as _, WM_COMMAND, safety::ID_RESUME, 0) };
    std::thread::sleep(Duration::from_millis(300));
    let still = safety::stopped();
    safety::resume_for_tests();
    assert!(still, "a posted Resume must not undo the kill switch");
    unsafe { PostMessageW(ui as _, WM_COMMAND, safety::ID_STOP, 0) };
    std::thread::sleep(Duration::from_millis(300));
    let stopped_by_message = safety::stopped();
    safety::resume_for_tests();
    assert!(
        stopped_by_message,
        "Stop (the safe direction) still works by message"
    );
    eprintln!("SEC-02 ok: posted Resume ignored, posted Stop works");
    core.stop().await;
}

// SEC-03 (hands off): an injected SendInput click on Allow is ignored too (the input source
// is IMO_INJECTED), which also proves the check sees the click's input message.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "hands off: one injected click on XtraOS's own dialog"]
async fn sec03_injected_click_on_allow_is_ignored() {
    #[repr(C)]
    struct Rect {
        l: i32,
        t: i32,
        r: i32,
        b: i32,
    }
    #[link(name = "user32")]
    extern "system" {
        fn GetWindowRect(h: *mut c_void, r: *mut Rect) -> i32;
        fn SetCursorPos(x: i32, y: i32) -> i32;
        fn mouse_event(f: u32, dx: u32, dy: u32, data: i32, extra: usize);
    }
    let _s = SERIAL.lock().await;
    policy_confirming_observe();
    let mut core = Core::start().await;
    let spoof = std::thread::spawn(|| {
        let d = wait_dialog();
        let allow = unsafe { GetDlgItem(d as _, safety::PROMPT_BUTTON_BASE) };
        let mut r = Rect {
            l: 0,
            t: 0,
            r: 0,
            b: 0,
        };
        unsafe { GetWindowRect(allow, &mut r) };
        unsafe {
            SetCursorPos((r.l + r.r) / 2, (r.t + r.b) / 2);
            mouse_event(0x0002, 0, 0, 0, 0); // left down (injected)
            mouse_event(0x0004, 0, 0, 0, 0); // left up
        }
        std::thread::sleep(Duration::from_millis(600));
        let still_open = unsafe { IsWindow(d as _) } != 0;
        safety::answer_prompt_for_tests(1);
        still_open
    });
    let r = core.call("list_windows", json!({})).await;
    let still_open = spoof.join().unwrap();
    eprintln!("SEC-03: dialog still open after an injected click: {still_open}");
    assert!(still_open, "injected input must not approve");
    assert_eq!(code(&r), "BLOCKED_BY_POLICY");
    core.stop().await;
}
