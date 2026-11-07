//! P2-M1 interactive tests (docs/08 P2-WF-*, P2-OBS-01, P2-BA-*): wait_for, scoped observe,
//! batch. They open the testbed and Calculator; one presses Ctrl+Alt+Esc. `#[ignore]`: run via
//! tests/phase2/p2m1-gate.ps1 -Interactive, hands off.
mod common;

use std::ffi::c_void;
use std::time::{Duration, Instant};

use common::{code, refs_by_name, tokens, Core, Testbed};
use serde_json::{json, Value};
use xtraos_core::safety;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[link(name = "user32")]
extern "system" {
    fn PostMessageW(h: *mut c_void, msg: u32, w: usize, l: isize) -> i32;
    fn SendMessageW(h: *mut c_void, msg: u32, w: usize, l: isize) -> isize;
    fn GetDlgItem(h: *mut c_void, id: i32) -> *mut c_void;
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn keybd_event(vk: u8, scan: u8, flags: u32, extra: usize);
}

fn text(v: &Value) -> String {
    v["text"].as_str().unwrap_or("").to_string()
}

fn resume_from_tray() {
    let ui = safety::ui_window().expect("safety UI");
    unsafe { PostMessageW(ui as _, 0x0111, safety::ID_RESUME, 0) };
    for _ in 0..50 {
        if !safety::stopped() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The human presses button `i` in the next core dialog.
fn human_answers(i: i32) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(10) {
            if let Some(&d) = safety::prompt_windows().first() {
                std::thread::sleep(Duration::from_millis(300));
                let b = unsafe { GetDlgItem(d as _, safety::PROMPT_BUTTON_BASE + i) };
                unsafe { SendMessageW(b, 0x00F5, 0, 0) };
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("no dialog appeared");
    })
}

async fn launch_calc(core: &mut Core) -> String {
    let w = core
        .ok("launch_app", json!({"app": "calc", "timeout_ms": 20000}))
        .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    w["ref"].as_str().unwrap().to_string()
}

async fn calc_refs(core: &mut Core, w: &str) -> std::collections::HashMap<String, String> {
    let o = core.ok("observe", json!({"window": w, "full": true})).await;
    refs_by_name(&text(&o))
}

// P2-WF-01: a window appears; wait_for finds it by title without a window ref.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn wf01_window_title_finds_new_window() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut child = std::process::Command::new(common::testbed_exe())
        .args(["300", "200"])
        .spawn()
        .unwrap();
    let r = core
        .ok(
            "wait_for",
            json!({"window_title": "xtraos testbed", "timeout_ms": 15000}),
        )
        .await;
    eprintln!("wf01: {r}");
    assert_eq!(r["met"], json!(["window_title"]));
    assert!(r["window"].as_str().unwrap().starts_with('w'));
    let _ = child.kill();
    let _ = child.wait();
    core.stop().await;
}

// P2-WF-02 + P2-WF-05: text appears after a background click; an unmet condition times out.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens Calculator"]
async fn wf02_wf05_text_contains_and_timeout() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let w = launch_calc(&mut core).await;
    let refs = calc_refs(&mut core, &w).await;
    core.ok("click", json!({"ref": refs["Clear"]})).await;
    core.ok("click", json!({"ref": refs["Seven"]})).await;
    let r = core
        .ok(
            "wait_for",
            json!({"window": w, "text_contains": "Display is 7", "timeout_ms": 5000}),
        )
        .await;
    eprintln!("wf02: {r}");
    assert!(r["ms"].as_u64().unwrap() < 1000, "{r}");
    let t0 = Instant::now();
    let e = core
        .call(
            "wait_for",
            json!({"window": w, "text_contains": "zzz never", "timeout_ms": 1000}),
        )
        .await;
    let ms = t0.elapsed().as_millis();
    assert_eq!(code(&e), "TIMEOUT", "{e}");
    assert!(e["error"]["message"]
        .as_str()
        .unwrap()
        .contains("text_contains"));
    assert!((1000..2000).contains(&ms), "{ms} ms");
    let _ = core.call("close_window", json!({"window": w})).await;
    core.stop().await;
}

// P2-WF-03 + P2-WF-04: idle on a static window; ref_gone and window_gone after it closes.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn wf03_wf04_idle_then_gone() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(120, 120);
    let w = tb.window();
    let o = core.ok("observe", json!({"window": w, "full": true})).await;
    let btn = refs_by_name(&text(&o))["btnA"].clone();
    let t0 = Instant::now();
    let r = core
        .ok("wait_for", json!({"window": w, "idle_ms": 600}))
        .await;
    let ms = t0.elapsed().as_millis();
    eprintln!("wf04 idle: {r} ({ms} ms)");
    assert!((550..2500).contains(&ms), "{ms}");
    tb.cmd("quit");
    let r = core
        .ok(
            "wait_for",
            json!({"window": w, "window_gone": true, "ref_gone": btn, "timeout_ms": 5000}),
        )
        .await;
    eprintln!("wf03 gone: {r}");
    assert_eq!(r["met"], json!(["ref_gone", "window_gone"]));
    drop(tb);
    core.stop().await;
}

// P2-OBS-01: scope_ref returns only the subtree.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn obs01_scope_ref_is_smaller() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let tb = Testbed::start(120, 120);
    let full = core
        .ok("observe", json!({"window": tb.window(), "full": true}))
        .await;
    let list = refs_by_name(&text(&full))["list"].clone();
    let scoped = core.ok("observe", json!({"scope_ref": list})).await;
    let st = text(&scoped);
    eprintln!(
        "scoped (~{} tokens vs ~{}):\n{st}",
        tokens(&st),
        tokens(&text(&full))
    );
    assert!(st.contains("Item 1"), "{st}");
    assert!(!st.contains("btnA"), "{st}");
    assert!(tokens(&st) < tokens(&text(&full)));
    core.stop().await;
}

// P2-BA-01: one batch call does the whole Calculator task.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens Calculator"]
async fn ba01_batch_runs_a_sequence_in_one_call() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let w = launch_calc(&mut core).await;
    let refs = calc_refs(&mut core, &w).await;
    let steps: Vec<Value> = ["Clear", "Seven", "Plus", "Five", "Equals"]
        .iter()
        .map(|n| json!({"tool": "click", "args": {"ref": refs[*n]}}))
        .chain([
            json!({"tool": "wait_for", "args": {"window": w, "text_contains": "Display is 12"}}),
        ])
        .collect();
    let r = core.ok("batch", json!({"steps": steps})).await;
    eprintln!("ba01: ok={} {}/{}", r["ok"], r["completed"], r["total"]);
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["completed"], 6);
    let log = std::fs::read_to_string(core.log.path()).unwrap();
    assert_eq!(
        log.matches("\"method\":\"batch:click\"").count(),
        5,
        "each step logged"
    );
    let _ = core.call("close_window", json!({"window": w})).await;
    core.stop().await;
}

// P2-BA-02 + P2-BA-04: a Send step needs the user's OK; denied → blocked, rest skipped.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn ba02_ba04_policy_applies_to_every_step() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(120, 120);
    let o = core
        .ok("observe", json!({"window": tb.window(), "full": true}))
        .await;
    let refs = refs_by_name(&text(&o));
    let human = human_answers(1); // Deny
    let r = core
        .ok(
            "batch",
            json!({"steps": [
                {"tool": "click", "args": {"ref": refs["btnA"]}},
                {"tool": "click", "args": {"ref": refs["send"]}},
                {"tool": "click", "args": {"ref": refs["btnB"]}}
            ]}),
        )
        .await;
    human.join().unwrap();
    eprintln!("ba02: {}", r["steps"]);
    assert_eq!(r["ok"], false);
    assert_eq!(r["steps"][1]["error"]["code"], "BLOCKED_BY_POLICY");
    assert_eq!(r["steps"][2]["skipped"], true);
    let st = tb.state();
    assert_eq!(st["clicks"]["btnA"], 1);
    assert_eq!(st["clicks"]["send"], 0, "nothing sent");
    assert_eq!(st["clicks"]["btnB"], 0, "rest skipped");
    // P2-BA-04 over the pipe: invalid steps run nothing
    let e = core
        .call(
            "batch",
            json!({"steps": [{"tool": "click", "args": {"ref": refs["btnA"]}}, {"tool": "batch"}]}),
        )
        .await;
    assert_eq!(code(&e), "INVALID_ARGS", "{e}");
    assert_eq!(tb.state()["clicks"]["btnA"], 1, "nothing executed");
    core.stop().await;
}

// P2-BA-03: the kill switch stops a batch mid-way; nothing stays pressed.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: drags and presses Ctrl+Alt+Esc"]
async fn ba03_kill_switch_stops_batch() {
    let _s = SERIAL.lock().await;
    struct ResumeOnDrop;
    impl Drop for ResumeOnDrop {
        fn drop(&mut self) {
            if safety::stopped() {
                resume_from_tray();
            }
        }
    }
    let _resume = ResumeOnDrop;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(120, 120);
    let [x, y, w, h] = tb.rects["drawPad"];
    let pts: Vec<[i32; 2]> = (0..300)
        .map(|i| {
            [
                x + 10 + (i % 100) * (w - 20) / 100,
                y + 10 + (i / 100) * (h - 20) / 3,
            ]
        })
        .collect();
    let killer = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(700));
        for vk in [0x11u8, 0x12, 0x1B] {
            unsafe { keybd_event(vk, 0, 0, 0) };
        }
        for vk in [0x1Bu8, 0x12, 0x11] {
            unsafe { keybd_event(vk, 0, 2, 0) };
        }
    });
    let (bx, by) = tb.center("btnA");
    let r = core
        .ok(
            "batch",
            json!({"steps": [
                {"tool": "mouse", "args": {"action": "path", "points": pts, "step_ms": 15, "window": tb.window()}},
                {"tool": "click", "args": {"x": bx, "y": by, "window": tb.window()}}
            ], "stop_on_error": false}),
        )
        .await;
    killer.join().unwrap();
    eprintln!("ba03: {}", r["steps"]);
    assert_eq!(r["steps"][0]["error"]["code"], "STOPPED");
    assert_eq!(
        r["steps"][1]["skipped"], true,
        "even with stop_on_error=false"
    );
    assert!(
        unsafe { GetAsyncKeyState(0x01) } as u16 & 0x8000 == 0,
        "button released"
    );
    resume_from_tray();
    assert_eq!(tb.state()["clicks"]["btnA"], 0);
    core.stop().await;
}
