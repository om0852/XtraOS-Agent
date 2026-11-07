//! M4 interactive tests (docs/08-test-plan.md P1-PER-*, P1-BG-*, P1-IN-01/07/08).
//! They open the testbed, Calculator and an Explorer window, so they are `#[ignore]`; run via
//! tests/phase1/m4-gate.ps1 -Interactive. Background tests also check that the cursor did not
//! move and that no input was injected — keep hands off while they run.
//! Opt-in: XTRAOS_TEST_BROWSERS=1 adds Firefox to the observe bench.
mod common;

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{code, refs_by_name, tokens, Core, Testbed};
use serde_json::{json, Value};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[repr(C)]
#[derive(Default, Clone, Copy, Debug, PartialEq)]
struct Pt {
    x: i32,
    y: i32,
}

#[repr(C)]
struct LastInput {
    size: u32,
    time: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct MouseIn {
    kind: u32,
    _pad: u32,
    dx: i32,
    dy: i32,
    data: u32,
    flags: u32,
    time: u32,
    _pad2: u32,
    extra: usize,
}

#[link(name = "user32")]
extern "system" {
    fn GetCursorPos(p: *mut Pt) -> i32;
    fn GetForegroundWindow() -> *mut c_void;
    fn GetAncestor(h: *mut c_void, flags: u32) -> *mut c_void;
    fn GetLastInputInfo(i: *mut LastInput) -> i32;
    fn SendInput(n: u32, inputs: *const MouseIn, size: i32) -> u32;
}

fn cursor() -> Pt {
    let mut p = Pt::default();
    unsafe { GetCursorPos(&mut p) };
    p
}

fn foreground() -> String {
    let h = unsafe { GetAncestor(GetForegroundWindow(), 2) };
    format!("w{}", h as usize)
}

fn last_input() -> u32 {
    let mut i = LastInput { size: 8, time: 0 };
    unsafe { GetLastInputInfo(&mut i) };
    i.time
}

/// Simulates a working user: 1-px relative mouse moves (net zero) until stopped.
fn simulate_user(stop: Arc<AtomicBool>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut d = 1;
        while !stop.load(Ordering::Relaxed) {
            let m = MouseIn {
                kind: 0,
                _pad: 0,
                dx: d,
                dy: 0,
                data: 0,
                flags: 0x0001, // MOUSEEVENTF_MOVE (relative)
                time: 0,
                _pad2: 0,
                extra: 0,
            };
            unsafe { SendInput(1, &m, std::mem::size_of::<MouseIn>() as i32) };
            d = -d;
            std::thread::sleep(Duration::from_millis(40));
        }
    })
}

fn text(v: &Value) -> String {
    v["text"].as_str().unwrap_or("").to_string()
}

async fn observe(core: &mut Core, window: &str, full: bool) -> Value {
    core.ok("observe", json!({"window": window, "full": full}))
        .await
}

/// Ref of the element whose automation id / name matches `q` (first find hit).
async fn find_ref(core: &mut Core, window: &str, q: &str) -> String {
    let r = core.ok("find", json!({"window": window, "query": q})).await;
    let t = text(&r);
    t.lines()
        .find(|l| l.starts_with('e'))
        .and_then(|l| l.split(' ').next())
        .unwrap_or_else(|| panic!("'{q}' not found in {window}: {t}"))
        .to_string()
}

async fn launch(core: &mut Core, app: &str, args: Value) -> (String, u64) {
    let w = core
        .ok(
            "launch_app",
            json!({"app": app, "args": args, "timeout_ms": 20000}),
        )
        .await;
    (
        w["ref"].as_str().unwrap().to_string(),
        w["pid"].as_u64().unwrap_or(0),
    )
}

async fn close(core: &mut Core, w: &str) {
    let _ = core
        .call("close_window", json!({"window": w, "timeout_ms": 5000}))
        .await;
}

fn median(v: &mut [u128]) -> u128 {
    v.sort_unstable();
    v[v.len() / 2]
}

// P1-PER-01 + P1-PER-06: token and latency budgets on real apps.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens Calculator, Explorer, testbed"]
async fn per01_per06_observe_budgets_on_real_apps() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let tb = Testbed::start(120, 120);
    let mut apps: Vec<(String, String, bool)> = vec![("testbed".into(), tb.window(), false)];
    let (calc, _) = launch(&mut core, "calc", json!([])).await;
    apps.push(("calculator".into(), calc.clone(), true));
    let dir = std::env::temp_dir();
    let (exp, _) = launch(&mut core, "explorer", json!([dir.to_string_lossy()])).await;
    apps.push(("explorer".into(), exp.clone(), true));
    if std::env::var("XTRAOS_TEST_BROWSERS").as_deref() == Ok("1") {
        let (ff, _) = launch(
            &mut core,
            "firefox",
            json!(["-new-window", "https://example.com"]),
        )
        .await;
        apps.push(("firefox".into(), ff, true));
    }
    tokio::time::sleep(Duration::from_millis(1500)).await; // let them render

    let mut toks = Vec::new();
    let mut results = Vec::new();
    for (name, w, _) in &apps {
        let first = observe(&mut core, w, true).await;
        let t = tokens(&text(&first));
        let mut ms = Vec::new();
        for _ in 0..10 {
            let t0 = Instant::now();
            observe(&mut core, w, true).await;
            ms.push(t0.elapsed().as_millis());
        }
        let p50 = median(&mut ms);
        eprintln!(
            "observe {name}: {} elements, ~{t} tokens, p50 {p50} ms, max {} ms",
            first["elements"],
            ms.last().unwrap()
        );
        toks.push(t);
        results.push((name.clone(), t, p50));
    }
    // Budget (docs/06): p50 <= 120 ms on a typical native-UIA app (Calculator). Large lists
    // (an Explorer folder with ~1,500 files, a 200-item MSAA list) are provider-bound and
    // slower (ADR 0016); for them this is a regression guard, not the budget.
    let mut fails = Vec::new();
    for (name, t, p50) in &results {
        if *t > 1500 {
            fails.push(format!("{name}: {t} tokens > cap 1500"));
        }
        let limit = if name == "calculator" { 120 } else { 1000 };
        if *p50 > limit {
            fails.push(format!("{name}: observe p50 {p50} ms > {limit} ms"));
        }
    }
    let mut sorted = toks.clone();
    sorted.sort_unstable();
    let med = sorted[sorted.len() / 2];
    eprintln!("observe median tokens: {med}");
    for (_, w, launched) in &apps {
        if *launched {
            close(&mut core, w).await;
        }
    }
    drop(tb);
    core.stop().await;
    assert!(med <= 400, "median {med} tokens > 400");
    assert!(fails.is_empty(), "{fails:?}");
}

// P1-PER-02 + P1-BG-01 + P1-BG-03: Calculator driven in the background; diff after a click.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens Calculator"]
async fn bg01_calculator_in_background_with_diff() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let tb = Testbed::start(120, 120);
    let (calc, _) = launch(&mut core, "calc", json!([])).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    // The user's window is the testbed; Calculator stays in the background.
    core.ok("focus_window", json!({"window": tb.window()}))
        .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    observe(&mut core, &calc, true).await;

    let mut refs = Vec::new();
    for id in [
        "clearButton",
        "num7Button",
        "num8Button",
        "plusButton",
        "num4Button",
        "equalButton",
    ] {
        refs.push(find_ref(&mut core, &calc, id).await);
    }
    // baseline for the diff
    observe(&mut core, &calc, false).await;

    let c0 = cursor();
    let in0 = last_input();
    let inj0 = xtraos_core::input::last_inject_tick();
    for r in &refs {
        let v = core.ok("click", json!({"ref": r})).await;
        assert_eq!(v["mode"], "background", "{v}");
    }
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(
        foreground(),
        tb.window(),
        "P1-BG-03: foreground must stay with the user"
    );
    assert_eq!(cursor(), c0, "P1-BG-01: cursor moved");
    assert_eq!(
        xtraos_core::input::last_inject_tick(),
        inj0,
        "P1-BG-01: the core injected input"
    );
    if last_input() != in0 {
        eprintln!("note: an input event happened meanwhile (the user?) — the core injected none");
    }

    let d = observe(&mut core, &calc, false).await;
    let dt = text(&d);
    eprintln!("diff after clicks (~{} tokens):\n{dt}", tokens(&dt));
    assert_eq!(d["diff"], true, "{dt}");
    assert!(dt.contains("82"), "display should show 82: {dt}");
    assert!(
        tokens(&dt) <= 80,
        "P1-PER-02: diff {} tokens > 80",
        tokens(&dt)
    );

    let disp = find_ref(&mut core, &calc, "CalculatorResults").await;
    let rt = core.ok("read_text", json!({"ref": disp})).await;
    assert!(text(&rt).contains("82"), "{rt}");
    close(&mut core, &calc).await;
    drop(tb);
    core.stop().await;
}

// P1-BG-02 / P1-IN-01: click(ref) on testbed (WinForms) buttons through native UIA.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn bg02_testbed_buttons_by_ref() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(120, 120);
    let o = observe(&mut core, &tb.window(), true).await;
    let t = text(&o);
    eprintln!("testbed observe:\n{t}");
    let refs = refs_by_name(&t);
    let c0 = cursor();
    let plan = [("btnA", 3), ("btnB", 4), ("btnC", 3)];
    for (name, n) in plan {
        let r = refs
            .get(name)
            .unwrap_or_else(|| panic!("{name} not in {t}"));
        for _ in 0..n {
            let v = core.ok("click", json!({"ref": r})).await;
            assert_eq!(v["mode"], "background", "{v}");
        }
    }
    let st = tb.state();
    for (name, n) in plan {
        assert_eq!(st["clicks"][name], n, "{st}");
    }
    assert_eq!(cursor(), c0, "cursor moved");
    core.stop().await;
}

// P1-IN-08: set_value through ValuePattern (no key events); P1-PER-07 passwords masked.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn in08_per07_set_value_and_password_masking() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(120, 120);
    let o = observe(&mut core, &tb.window(), true).await;
    let refs = refs_by_name(&text(&o));
    let field = refs["text"].clone();
    let value = "background नमस्ते 👋";
    let v = core
        .ok("set_value", json!({"ref": field, "value": value}))
        .await;
    assert_eq!(v["mode"], "background", "{v}");
    let st = tb.state();
    assert_eq!(st["text"], value, "{st}");
    assert_eq!(
        st["keys"].as_array().unwrap().len(),
        0,
        "no key events: {st}"
    );

    // password: value never shown, never read, never set
    tb.cmd("fillpassword");
    let o = observe(&mut core, &tb.window(), true).await;
    let t = text(&o);
    assert!(!t.contains("hunter2"), "password leaked in observe: {t}");
    let pw = refs_by_name(&t)["password"].clone();
    assert!(
        t.lines()
            .any(|l| l.starts_with(&format!("{pw} ")) && l.contains("***")),
        "password line not masked: {t}"
    );
    let r = core.ok("read_text", json!({"ref": pw})).await;
    assert_eq!(r["text"], "***", "{r}");
    let r = core
        .call("set_value", json!({"ref": pw, "value": "x"}))
        .await;
    // M5: policy denies credential_entry before the action runs (ADR 0017).
    assert_eq!(code(&r), "BLOCKED_BY_POLICY", "{r}");
    let f = core
        .ok("find", json!({"window": tb.window(), "query": "hunter2"}))
        .await;
    assert_eq!(f["matches"], 0, "{f}");
    core.stop().await;
}

// P1-PER-03: opaque detection.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn per03_opaque_window_is_flagged() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let exe = common::testbed_exe();
    let (w, pid) = launch(
        &mut core,
        exe.to_string_lossy().trim_start_matches(r"\\?\"),
        json!(["200", "160", "--opaque"]),
    )
    .await;
    let o = observe(&mut core, &w, true).await;
    eprintln!("opaque observe:\n{}", text(&o));
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .output();
    assert_eq!(o["opaque"], true, "{o}");
    // Informational: an Electron app, if one is open (Claude desktop / VS Code).
    let ws = core.ok("list_windows", json!({})).await;
    for x in ws["windows"].as_array().unwrap() {
        let p = x["process"].as_str().unwrap_or("").to_lowercase();
        if p == "claude.exe" || p == "code.exe" {
            let r = core
                .call("observe", json!({"window": x["ref"], "full": true}))
                .await;
            eprintln!(
                "electron {p}: elements={} opaque={}",
                r["result"]["elements"], r["result"]["opaque"]
            );
            break;
        }
    }
    core.stop().await;
}

// P1-PER-04: screenshot → screen mapping.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn per04_screenshot_maps_back_to_screen() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let tb = Testbed::start(120, 120);
    let s = core
        .ok("screenshot", json!({"window": tb.window(), "scale": 0.5}))
        .await;
    let b64 = s["png_base64"].as_str().unwrap();
    let png = decode_b64(b64);
    let dec = png::Decoder::new(std::io::Cursor::new(png));
    let mut rd = dec.read_info().unwrap();
    let mut buf = vec![0; rd.output_buffer_size().unwrap()];
    let info = rd.next_frame(&mut buf).unwrap();
    let (w, h) = (info.width as usize, info.height as usize);
    // LightSteelBlue pad = (176, 196, 222)
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) * 3;
            let (r, g, b) = (buf[i] as i32, buf[i + 1] as i32, buf[i + 2] as i32);
            if (r - 176).abs() <= 3 && (g - 196).abs() <= 3 && (b - 222).abs() <= 3 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    assert!(x0 != usize::MAX, "pad colour not found in the screenshot");
    let scale = s["scale"].as_f64().unwrap();
    let ox = s["origin"][0].as_f64().unwrap();
    let oy = s["origin"][1].as_f64().unwrap();
    let [px, py, pw, ph] = tb.rects["pad"];
    let mapped = (ox + x0 as f64 / scale, oy + y0 as f64 / scale);
    let mapped_w = (x1 - x0 + 1) as f64 / scale;
    eprintln!(
        "pad screen ({px},{py} {pw}x{ph}) vs mapped ({:.1},{:.1} w {:.1}); {}x{} png {} bytes",
        mapped.0, mapped.1, mapped_w, w, h, s["bytes"]
    );
    // ≤ 1 screenshot pixel = ≤ 1/scale screen pixels
    let tol = 1.0 / scale;
    assert!((mapped.0 - px as f64).abs() <= tol, "x off");
    assert!((mapped.1 - py as f64).abs() <= tol, "y off");
    assert!((mapped_w - pw as f64).abs() <= tol + 1.0, "width off");
    core.stop().await;
}

fn decode_b64(s: &str) -> Vec<u8> {
    let val = |c: u8| -> u32 {
        match c {
            b'A'..=b'Z' => (c - b'A') as u32,
            b'a'..=b'z' => (c - b'a' + 26) as u32,
            b'0'..=b'9' => (c - b'0' + 52) as u32,
            b'+' => 62,
            _ => 63,
        }
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for chunk in s.as_bytes().chunks(4) {
        let pad = chunk.iter().filter(|&&c| c == b'=').count();
        let n = chunk
            .iter()
            .map(|&c| if c == b'=' { 0 } else { val(c) })
            .fold(0u32, |a, v| (a << 6) | v);
        let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        out.extend_from_slice(&bytes[..3 - pad]);
    }
    out
}

// P1-PER-05: stale refs.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn per05_stale_ref_after_window_closes() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let tb = Testbed::start(120, 120);
    let o = observe(&mut core, &tb.window(), true).await;
    let r = refs_by_name(&text(&o))["btnA"].clone();
    drop(tb); // closes the window
    tokio::time::sleep(Duration::from_millis(300)).await;
    let e = core.call("click", json!({"ref": r})).await;
    assert_eq!(code(&e), "STALE_REF", "{e}");
    let tb2 = Testbed::start(120, 120);
    let o2 = observe(&mut core, &tb2.window(), true).await;
    assert!(refs_by_name(&text(&o2)).contains_key("btnA"));
    core.stop().await;
}

// P1-BG-04: real input waits for the user, then refuses; nothing is clicked.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn bg04_real_input_waits_while_user_is_active() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(120, 120);
    let (x, y) = tb.center("btnA");
    let stop = Arc::new(AtomicBool::new(false));
    let user = simulate_user(stop.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let t0 = Instant::now();
    let r = core
        .call(
            "click",
            json!({"x": x, "y": y, "window": tb.window(), "idle_ms": 800, "timeout_ms": 1500}),
        )
        .await;
    let waited = t0.elapsed().as_millis();
    stop.store(true, Ordering::Relaxed);
    user.join().unwrap();
    assert_eq!(code(&r), "USER_ACTIVE", "{r}");
    assert!(
        waited >= 1400,
        "should wait for the user first ({waited} ms)"
    );
    assert_eq!(tb.state()["clicks"]["btnA"], 0, "nothing may be clicked");
    // once the user is idle, the same click goes through
    let ok = core
        .ok("click", json!({"x": x, "y": y, "window": tb.window()}))
        .await;
    assert_eq!(ok["mode"], "input");
    assert_eq!(tb.state()["clicks"]["btnA"], 1);
    core.stop().await;
}

// P1-BG-05: a real click puts the cursor back.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: moves the mouse"]
async fn bg05_real_click_restores_cursor() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(120, 120);
    let (x, y) = tb.center("btnB");
    let before = cursor();
    core.ok("click", json!({"x": x, "y": y, "window": tb.window()}))
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let after = cursor();
    assert!(
        (after.x - before.x).abs() <= 2 && (after.y - before.y).abs() <= 2,
        "cursor not restored: {before:?} -> {after:?}"
    );
    core.ok(
        "click",
        json!({"x": x, "y": y, "window": tb.window(), "restore_cursor": false}),
    )
    .await;
    let at = cursor();
    assert!((at.x - x).abs() <= 2 && (at.y - y).abs() <= 2, "{at:?}");
    assert_eq!(tb.state()["clicks"]["btnB"], 2);
    core.stop().await;
}

// P1-BG-06: scroll(ref) scrolls a list through its Scroll pattern, without the wheel.
// (Items outside the visible part of a virtualised list are not in the UIA tree at all, so
// "scroll item 150 into view" needs the ItemContainer pattern: backlog.)
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn bg06_scroll_ref_without_wheel() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start(120, 120);
    let o = observe(&mut core, &tb.window(), true).await;
    let list = refs_by_name(&text(&o))["list"].clone();
    let c0 = cursor();
    let in0 = last_input();
    let inj0 = xtraos_core::input::last_inject_tick();
    let v = core.ok("scroll", json!({"ref": list, "dy": 10})).await;
    eprintln!("scroll(ref): {v}");
    assert_eq!(v["mode"], "background", "{v}");
    let st = tb.state();
    assert_eq!(st["listTop"], 10, "{st}");
    // An item without ScrollItem (WinForms list items via MSAA) is refused, not wheeled.
    let o = observe(&mut core, &tb.window(), true).await;
    let item = refs_by_name(&text(&o))["Item 12"].clone();
    let r = core.call("scroll", json!({"ref": item})).await;
    assert_eq!(code(&r), "INVALID_ARGS", "{r}");
    assert_eq!(cursor(), c0);
    let _ = in0;
    assert_eq!(
        xtraos_core::input::last_inject_tick(),
        inj0,
        "the core injected input"
    );
    core.stop().await;
}

// P1-IN-07: drag a file from Explorer (located via observe) onto the testbed drop zone.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: drags with the real mouse"]
async fn in07_drag_file_from_explorer() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let dir = std::env::temp_dir().join(format!("xtraos-drop-{}", common::rand_hex(3)));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("xtraos-drop-test.txt"), "hello").unwrap();
    let mut tb = Testbed::start(60, 120);
    let (exp, _) = launch(&mut core, "explorer", json!([dir.to_string_lossy()])).await;
    core.ok(
        "window_bounds",
        json!({"window": exp, "x": 900, "y": 120, "w": 800, "h": 520}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let item = find_ref(&mut core, &exp, "xtraos-drop-test").await;
    let (tx, ty) = tb.center("dropTarget");
    let v = core
        .ok(
            "drag",
            json!({"from_ref": item, "to_x": tx, "to_y": ty, "to_window": tb.window(),
                   "steps": 30, "step_ms": 15, "hold_ms": 200}),
        )
        .await;
    eprintln!("drag: {v}");
    let st = tb.state();
    close(&mut core, &exp).await;
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(st["dropped"], "file:xtraos-drop-test.txt", "{st}");
    core.stop().await;
}
