//! P3-M3 touch / pen (docs/08 P3-TP-*, ADR 0036). TP-01..03 inject real pointer input
//! (hands off: run from p2m7-gate.ps1 -Interactive). TP-04/05 inject nothing (a refused or
//! denied gesture) and run in the automated gate.
mod common;

use std::time::{Duration, Instant};

use common::{code, Core, Testbed};
use serde_json::{json, Value};
use xtraos_core::safety;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[link(name = "user32")]
extern "system" {
    fn SendMessageW(h: *mut std::ffi::c_void, msg: u32, w: usize, l: isize) -> isize;
    fn GetDlgItem(h: *mut std::ffi::c_void, id: i32) -> *mut std::ffi::c_void;
    fn GetWindowTextW(h: *mut std::ffi::c_void, buf: *mut u16, n: i32) -> i32;
}

fn private_policy() {
    let home = std::env::temp_dir().join(format!("xtraos-p3m3-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    safety::set_policy_path(home.join("policy.json"));
}

fn human_answers(i: i32, wait: Duration) -> std::thread::JoinHandle<Option<String>> {
    std::thread::spawn(move || {
        let t0 = Instant::now();
        while t0.elapsed() < wait {
            if let Some(&d) = safety::prompt_windows().first() {
                std::thread::sleep(Duration::from_millis(300));
                let mut buf = [0u16; 2048];
                let st = unsafe { GetDlgItem(d as _, safety::PROMPT_TEXT_ID) };
                let n = unsafe { GetWindowTextW(st, buf.as_mut_ptr(), 2048) };
                let b = unsafe { GetDlgItem(d as _, safety::PROMPT_BUTTON_BASE + i) };
                unsafe { SendMessageW(b, 0x00F5, 0, 0) };
                return Some(String::from_utf16_lossy(&buf[..n.max(0) as usize]));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    })
}

// P3-TP-04: a 3-finger swipe is a Windows gesture → asks (Deny → nothing injected); bad
// actions are refused before any injection.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "shows a core approval dialog (answered by the test)"]
async fn tp04_gesture_asks_nothing_injected() {
    let _s = SERIAL.lock().await;
    private_policy();
    let mut core = Core::start().await;
    let tb = Testbed::start(300, 200);
    let [x, y, w, h] = tb.rects["btnA"];
    let before = xtraos_core::input::last_inject_tick();
    let human = human_answers(1, Duration::from_secs(10)); // Deny
    let r = core
        .call(
            "touch",
            json!({"action": "swipe", "fingers": 3, "x": x + w / 2, "y": y + h / 2, "to_x": x + w / 2, "to_y": y - 200}),
        )
        .await;
    let msg = human.join().unwrap().expect("a dialog");
    eprintln!("TP-04 dialog: {}", msg.replace('\n', " "));
    assert!(msg.contains("system_settings"), "{msg}");
    assert_eq!(code(&r), "BLOCKED_BY_POLICY", "{r}");
    assert_eq!(
        xtraos_core::input::last_inject_tick(),
        before,
        "nothing injected"
    );
    // (No tap aimed at an XtraOS window here: the banner is hidden between actions, so the
    // point would land on whatever window is there. check_point() refuses XtraOS and admin
    // windows before any injection; ED-03 covers the viewer.)
    let bad = core
        .call("touch", json!({"action": "rotate", "x": 1, "y": 1}))
        .await;
    assert_eq!(code(&bad), "INVALID_ARGS");
    drop(tb);
    core.stop().await;
}

// P3-TP-01: a touch tap on a testbed button clicks it (hands off).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "hands off: real touch input"]
async fn tp01_touch_tap_clicks() {
    let _s = SERIAL.lock().await;
    private_policy();
    let mut core = Core::start().await;
    let mut tb = Testbed::start(300, 200);
    core.ok("focus_window", json!({"window": tb.window()}))
        .await;
    let (cx, cy) = tb.center("btnA");
    let r = core
        .ok("touch", json!({"action": "tap", "x": cx, "y": cy}))
        .await;
    eprintln!("TP-01: {}", r["text"]);
    std::thread::sleep(Duration::from_millis(300));
    let st: Value = tb.state();
    assert_eq!(st["clicks"]["btnA"], 1, "{st}");
    drop(tb);
    core.stop().await;
}

// P3-TP-02: a pen stroke on the testbed's draw pad arrives as a continuous path (hands off).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "hands off: real pen input"]
async fn tp02_pen_draws_a_path() {
    let _s = SERIAL.lock().await;
    private_policy();
    let mut core = Core::start().await;
    let mut tb = Testbed::start(300, 200);
    core.ok("focus_window", json!({"window": tb.window()}))
        .await;
    let [x, y, w, h] = tb.rects["drawPad"];
    let pts: Vec<Value> = (0..10)
        .map(|i| {
            json!([
                x + 10 + i * (w - 20) / 10,
                y + h / 2 + if i % 2 == 0 { -10 } else { 10 }
            ])
        })
        .collect();
    let r = core
        .ok(
            "pen",
            json!({"action": "draw", "points": pts, "pressure": 700}),
        )
        .await;
    eprintln!("TP-02: {}", r["text"]);
    std::thread::sleep(Duration::from_millis(300));
    let st = tb.state();
    eprintln!("TP-02 path: {}", st["path"]);
    assert!(st["path"]["count"].as_u64().unwrap_or(0) > 5, "{st}");
    drop(tb);
    core.stop().await;
}

// P3-TP-03: the kill switch during a long press lifts the contact and stops (hands off).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "hands off: real touch input + kill switch"]
async fn tp03_kill_switch_lifts_touch() {
    let _s = SERIAL.lock().await;
    private_policy();
    let mut core = Core::start().await;
    let tb = Testbed::start(300, 200);
    let (cx, cy) = tb.center("btnB");
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(400));
        safety::stop();
    });
    let r = core
        .call(
            "touch",
            json!({"action": "long_press", "x": cx, "y": cy, "duration_ms": 3000}),
        )
        .await;
    assert_eq!(code(&r), "STOPPED", "{r}");
    let ui = safety::ui_window().expect("safety UI");
    #[link(name = "user32")]
    extern "system" {
        fn PostMessageW(h: *mut std::ffi::c_void, msg: u32, w: usize, l: isize) -> i32;
    }
    unsafe { PostMessageW(ui as _, 0x0111, safety::ID_RESUME, 0) };
    std::thread::sleep(Duration::from_millis(200));
    drop(tb);
    core.stop().await;
}
