//! M2 interactive end-to-end tests (docs/08-test-plan.md P1-IN-*, P1-LOG-02).
//! Real input → real pipe → core → testbed window. They move the mouse and type, so they are
//! `#[ignore]` and run only via `cargo test --test input_e2e -- --ignored --test-threads=1`
//! (tests/phase1/m2-gate.ps1 -Interactive). Every action targets the testbed window handle,
//! so the guards refuse to touch anything else. Keep hands off the mouse while they run.
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader as TokioBufReader};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use xtraos_core::{actionlog::ActionLog, input, ipc, winsec};

// Async mutex: tests hold it across awaits so only one drives the real mouse at a time.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[link(name = "user32")]
extern "system" {
    fn SetCursorPos(x: i32, y: i32) -> i32;
    fn GetAsyncKeyState(vk: i32) -> i16;
    fn FindWindowW(class: *const u16, title: *const u16) -> *mut std::ffi::c_void;
}

fn held(vk: i32) -> bool {
    (unsafe { GetAsyncKeyState(vk) } as u16 & 0x8000) != 0
}

fn assert_nothing_stuck() {
    for (vk, name) in [
        (0x01, "LButton"),
        (0x02, "RButton"),
        (0x10, "Shift"),
        (0x11, "Ctrl"),
        (0x12, "Alt"),
        (0x5B, "Win"),
    ] {
        assert!(!held(vk), "{name} is still held after the action");
    }
}

// ---------------------------------------------------------------------------------- testbed

struct Testbed {
    child: Child,
    stdin: ChildStdin,
    out: BufReader<ChildStdout>,
    hwnd: u64,
    rects: HashMap<String, [i32; 4]>,
}

impl Testbed {
    fn start() -> Self {
        let exe = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testbed/bin/testbed.exe");
        assert!(
            exe.exists(),
            "build the testbed first: testbed/build.ps1 ({})",
            exe.display()
        );
        let mut child = Command::new(exe)
            .args(["120", "120"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let out = BufReader::new(child.stdout.take().unwrap());
        let mut tb = Testbed {
            child,
            stdin,
            out,
            hwnd: 0,
            rects: HashMap::new(),
        };
        let ready = tb.line();
        assert_eq!(ready["ready"], true, "testbed did not start: {ready}");
        std::thread::sleep(Duration::from_millis(400)); // let it paint
        let r = tb.cmd("rects");
        tb.hwnd = r["hwnd"].as_u64().unwrap();
        for (k, v) in r["controls"].as_object().unwrap() {
            let a: Vec<i32> = v
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_i64().unwrap() as i32)
                .collect();
            tb.rects.insert(k.clone(), [a[0], a[1], a[2], a[3]]);
        }
        tb
    }

    fn line(&mut self) -> Value {
        let mut s = String::new();
        self.out.read_line(&mut s).unwrap();
        serde_json::from_str(&s).unwrap_or_else(|e| panic!("bad testbed line {s:?}: {e}"))
    }

    fn cmd(&mut self, c: &str) -> Value {
        writeln!(self.stdin, "{c}").unwrap();
        self.stdin.flush().unwrap();
        self.line()
    }

    fn state(&mut self) -> Value {
        std::thread::sleep(Duration::from_millis(150)); // let the UI process queued input
        self.cmd("state")
    }

    fn center(&self, name: &str) -> (i32, i32) {
        let [x, y, w, h] = self.rects[name];
        (x + w / 2, y + h / 2)
    }

    fn rect(&self, name: &str) -> [i32; 4] {
        self.rects[name]
    }
}

impl Drop for Testbed {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "quit");
        let _ = self.stdin.flush();
        for _ in 0..30 {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = self.child.kill();
    }
}

// ------------------------------------------------------------------------------- core + pipe

struct Core {
    server: Option<ipc::ServerHandle>,
    log: Arc<ActionLog>,
    client: Client,
    dir: PathBuf,
}

struct Client {
    rd: TokioBufReader<tokio::io::ReadHalf<NamedPipeClient>>,
    wr: tokio::io::WriteHalf<NamedPipeClient>,
    id: u64,
}

fn rand_hex(n: usize) -> String {
    winsec::random_bytes(n)
        .unwrap()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Core {
    async fn start() -> Self {
        input::init_dpi();
        let pipe = format!(r"\\.\pipe\xtraos-in-{}", rand_hex(6));
        let token = rand_hex(32);
        let dir = std::env::temp_dir().join(format!("xtraos-in-{}", rand_hex(4)));
        let log = Arc::new(ActionLog::create(&dir).unwrap());
        let server = ipc::start(ipc::ServerConfig {
            pipe_name: pipe.clone(),
            token: token.clone(),
            log: log.clone(),
        })
        .unwrap();
        let p = ClientOptions::new().open(&pipe).unwrap();
        let (rd, wr) = tokio::io::split(p);
        let mut client = Client {
            rd: TokioBufReader::new(rd),
            wr,
            id: 0,
        };
        let r = client
            .call("pair", json!({"token": token, "client": "input-e2e"}))
            .await;
        assert!(r.get("result").is_some(), "{r}");
        Core {
            server: Some(server),
            log,
            client,
            dir,
        }
    }

    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.client.call(method, params).await
    }

    async fn ok(&mut self, method: &str, params: Value) -> Value {
        let r = self.call(method, params.clone()).await;
        assert!(r.get("result").is_some(), "{method} {params} failed: {r}");
        r["result"].clone()
    }

    async fn stop(mut self) {
        self.server.take().unwrap().shutdown().await.unwrap();
        std::fs::remove_dir_all(&self.dir).ok();
    }
}

impl Client {
    async fn call(&mut self, method: &str, params: Value) -> Value {
        self.id += 1;
        let msg = json!({"jsonrpc":"2.0","id":self.id,"method":method,"params":params});
        self.wr
            .write_all(format!("{msg}\n").as_bytes())
            .await
            .unwrap();
        let mut s = String::new();
        tokio::time::timeout(Duration::from_secs(20), self.rd.read_line(&mut s))
            .await
            .expect("core reply timeout")
            .unwrap();
        serde_json::from_str(&s).unwrap()
    }
}

fn code(v: &Value) -> &str {
    v.pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or("")
}

async fn click(core: &mut Core, tb: &Testbed, name: &str, extra: Value) -> Value {
    let (x, y) = tb.center(name);
    let mut p = json!({"x": x, "y": y, "window": tb.hwnd});
    if let (Some(o), Some(e)) = (p.as_object_mut(), extra.as_object()) {
        o.extend(e.clone());
    }
    core.call("click", p).await
}

/// Click that must succeed; on failure the core's error (code + message) is in the panic.
async fn click_ok(core: &mut Core, tb: &Testbed, name: &str, extra: Value) {
    let r = click(core, tb, name, extra.clone()).await;
    assert!(
        r.get("result").is_some(),
        "click {name} {extra} failed: {r}"
    );
}

// ------------------------------------------------------------------------------------ tests

// P1-IN-01
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: moves the mouse over the testbed"]
async fn in01_ten_clicks_land_on_the_right_buttons() {
    let _s = SERIAL.lock().await;
    let mut tb = Testbed::start();
    let mut core = Core::start().await;
    let mut expected = json!({"btnA": 0, "btnB": 0, "btnC": 0});
    for (name, n) in [("btnA", 4), ("btnB", 3), ("btnC", 3)] {
        for i in 0..n {
            let r = click(&mut core, &tb, name, json!({})).await;
            assert!(
                r.get("result").is_some(),
                "{name} click #{} failed: {r}",
                i + 1
            );
            expected[name] = json!(expected[name].as_i64().unwrap() + 1);
            let st = tb.state();
            assert_eq!(
                st["clicks"],
                expected,
                "after {name} click #{} (core said {r})",
                i + 1
            );
            tokio::time::sleep(Duration::from_millis(550)).await; // avoid double-click merging
        }
    }
    let st = tb.state();
    assert_eq!(
        st["clicks"],
        json!({"btnA": 4, "btnB": 3, "btnC": 3}),
        "{st}"
    );
    assert_nothing_stuck();
    core.stop().await;
}

// P1-IN-02
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: moves the mouse over the testbed"]
async fn in02_double_right_and_triple_click() {
    let _s = SERIAL.lock().await;
    let mut tb = Testbed::start();
    let mut core = Core::start().await;

    click_ok(&mut core, &tb, "pad", json!({"clicks": 2})).await;
    assert_eq!(tb.state()["pad"], json!(["Left:1", "Left:2"]));
    tb.cmd("reset");
    tokio::time::sleep(Duration::from_millis(600)).await;

    click_ok(&mut core, &tb, "pad", json!({"button": "right"})).await;
    assert_eq!(tb.state()["pad"], json!(["Right:1"]));
    tb.cmd("reset");
    tokio::time::sleep(Duration::from_millis(600)).await;

    click_ok(&mut core, &tb, "pad", json!({"clicks": 3})).await;
    let pad = tb.state()["pad"].clone();
    assert_eq!(pad.as_array().unwrap().len(), 3, "{pad}");
    assert_eq!(pad[0], "Left:1");
    assert_eq!(pad[1], "Left:2");
    assert_nothing_stuck();
    core.stop().await;
}

// P1-IN-03
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: types into the testbed"]
async fn in03_type_unicode_hindi_emoji_enter_tab() {
    let _s = SERIAL.lock().await;
    let mut tb = Testbed::start();
    let mut core = Core::start().await;

    click_ok(&mut core, &tb, "text", json!({})).await; // activates + focuses
    let text = "Hello नमस्ते 🙂 ✓";
    core.ok("type", json!({"window": tb.hwnd, "text": text}))
        .await;
    assert_eq!(tb.state()["text"], text);

    click_ok(&mut core, &tb, "multi", json!({})).await;
    core.ok(
        "type",
        json!({"window": tb.hwnd, "text": "line1\nline2\tX"}),
    )
    .await;
    assert_eq!(tb.state()["multi"], "line1\r\nline2\tX");
    assert_nothing_stuck();
    core.stop().await;
}

// P1-IN-04 + P1-IN-12
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: types into the testbed"]
async fn in04_key_chords_and_nothing_stuck() {
    let _s = SERIAL.lock().await;
    let mut tb = Testbed::start();
    let mut core = Core::start().await;

    click_ok(&mut core, &tb, "text", json!({})).await;
    core.ok("type", json!({"window": tb.hwnd, "text": "abc"}))
        .await;
    core.ok(
        "press_keys",
        json!({"window": tb.hwnd, "keys": ["Ctrl+A", "Backspace"]}),
    )
    .await;
    core.ok(
        "press_keys",
        json!({"window": tb.hwnd, "keys": ["Ctrl+Shift+K", "F5"]}),
    )
    .await;
    let st = tb.state();
    assert_eq!(st["text"], "", "Ctrl+A then Backspace must clear: {st}");
    let keys: Vec<&str> = st["keys"]
        .as_array()
        .unwrap()
        .iter()
        .map(|k| k.as_str().unwrap())
        .collect();
    for want in ["Ctrl+A", "Back", "Ctrl+Shift+K", "F5"] {
        assert!(keys.contains(&want), "missing {want} in {keys:?}");
    }
    assert_nothing_stuck();
    core.stop().await;
}

// P1-IN-05
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: moves the mouse over the testbed"]
async fn in05_scroll_list_down_and_up() {
    let _s = SERIAL.lock().await;
    let mut tb = Testbed::start();
    let mut core = Core::start().await;
    let (x, y) = tb.center("list");
    core.ok(
        "scroll",
        json!({"x": x, "y": y, "window": tb.hwnd, "dy": 3}),
    )
    .await;
    let down = tb.state()["listTop"].as_i64().unwrap();
    assert!(down > 0, "list did not scroll down (top={down})");
    core.ok(
        "scroll",
        json!({"x": x, "y": y, "window": tb.hwnd, "dy": -3}),
    )
    .await;
    let up = tb.state()["listTop"].as_i64().unwrap();
    assert!(up < down, "list did not scroll back up ({up} vs {down})");
    core.stop().await;
}

// P1-IN-06: real OLE drag-and-drop + a free mouse path.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: drags inside the testbed"]
async fn in06_ole_drag_and_drop_and_mouse_path() {
    let _s = SERIAL.lock().await;
    let mut tb = Testbed::start();
    let mut core = Core::start().await;

    let (fx, fy) = tb.center("dragSource");
    let (tx, ty) = tb.center("dropTarget");
    core.ok(
        "drag",
        json!({"from_x": fx, "from_y": fy, "to_x": tx, "to_y": ty, "window": tb.hwnd, "steps": 25}),
    )
    .await;
    assert_eq!(tb.state()["dropped"], "payload-42");

    let [x, y, w, h] = tb.rect("drawPad");
    let pts: Vec<[i32; 2]> = (0..12)
        .map(|i| {
            [
                x + 20 + i * (w - 40) / 11,
                y + h / 2 + if i % 2 == 0 { -15 } else { 15 },
            ]
        })
        .collect();
    core.ok(
        "mouse",
        json!({"action": "path", "points": pts, "window": tb.hwnd}),
    )
    .await;
    let path = tb.state()["path"].clone();
    assert!(path["count"].as_i64().unwrap() >= 12, "{path}");
    assert_eq!(path["first"], json!(pts[0]), "{path}");
    assert_eq!(path["last"], json!(pts[11]), "{path}");
    assert_nothing_stuck();
    core.stop().await;
}

// P1-IN-09
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn in09_keyboard_refused_when_target_is_not_foreground() {
    let _s = SERIAL.lock().await;
    let mut tb = Testbed::start();
    let mut core = Core::start().await;
    let class: Vec<u16> = "Shell_TrayWnd\0".encode_utf16().collect();
    let taskbar = unsafe { FindWindowW(class.as_ptr(), std::ptr::null()) } as u64;
    assert_ne!(taskbar, 0, "taskbar not found");
    let r = core
        .call(
            "type",
            json!({"window": taskbar, "text": "should-not-appear"}),
        )
        .await;
    assert_eq!(code(&r), "NEEDS_HUMAN", "{r}");
    let r = core
        .call("press_keys", json!({"window": taskbar, "keys": ["Ctrl+A"]}))
        .await;
    assert_eq!(code(&r), "NEEDS_HUMAN", "{r}");
    let st = tb.state();
    assert_eq!(st["text"], "");
    assert_eq!(st["keys"], json!([]));
    core.stop().await;
}

// P1-IN-10: the "user" moves the mouse mid-drag → abort, button released, drag incomplete.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: drags inside the testbed"]
async fn in10_user_moving_the_mouse_aborts_and_releases() {
    let _s = SERIAL.lock().await;
    let mut tb = Testbed::start();
    let mut core = Core::start().await;
    let [x, y, w, h] = tb.rect("drawPad");
    // Simulated user: a hand on the mouse moves continuously (a stream of positions), not a
    // single instantaneous jump, which a following automated move could overwrite unseen.
    let user = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        for i in 0..15 {
            let ok = unsafe { SetCursorPos(x + w / 2 + i * 3, y - 60 - i * 2) };
            assert_ne!(ok, 0, "SetCursorPos failed");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    });
    let r = core
        .call(
            "drag",
            json!({"from_x": x + 10, "from_y": y + h / 2, "to_x": x + w - 10, "to_y": y + h / 2,
                             "window": tb.hwnd, "steps": 80, "step_ms": 15}),
        )
        .await;
    user.await.unwrap();
    assert_eq!(code(&r), "USER_ACTIVE", "{r}");
    assert_nothing_stuck();
    let path = tb.state()["path"].clone();
    assert!(
        path["count"].as_i64().unwrap() < 80,
        "drag should have stopped early: {path}"
    );
    core.stop().await;
}

// P1-IN-11
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn in11_point_outside_target_window_is_refused() {
    let _s = SERIAL.lock().await;
    let mut tb = Testbed::start();
    let mut core = Core::start().await;
    let [x, y, w, _] = tb.rect("btnC");
    let outside_x = x + w + 600; // beyond the testbed's right edge
    let r = core
        .call(
            "click",
            json!({"x": outside_x, "y": y + 10, "window": tb.hwnd}),
        )
        .await;
    assert_eq!(code(&r), "TARGET_OBSCURED", "{r}");
    assert_eq!(
        tb.state()["clicks"],
        json!({"btnA": 0, "btnB": 0, "btnC": 0})
    );
    core.stop().await;
}

// P1-LOG-02: typed text (e.g. a password) never reaches the action log.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: types into the testbed"]
async fn log02_typed_password_is_never_logged() {
    let _s = SERIAL.lock().await;
    let mut tb = Testbed::start();
    let mut core = Core::start().await;
    click_ok(&mut core, &tb, "password", json!({})).await;
    let secret = "SECRET-xtra-7";
    let r = core
        .ok("type", json!({"window": tb.hwnd, "text": secret}))
        .await;
    assert!(
        !r.to_string().contains("SECRET"),
        "result must not echo the text: {r}"
    );
    assert_eq!(tb.state()["passwordLen"], secret.len());
    let log = std::fs::read_to_string(core.log.path()).unwrap();
    assert!(
        !log.contains("SECRET"),
        "typed text leaked into the action log"
    );
    assert!(
        log.contains("\"method\":\"type\""),
        "the type action itself is logged"
    );
    core.stop().await;
}

// Clipboard round trip; restores the user's text clipboard. Skips if it holds non-text.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: touches the clipboard (restored afterwards)"]
async fn clipboard_set_get_and_restore() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let original = core.ok("clipboard", json!({"op": "get"})).await["text"].clone();
    let Some(orig) = original.as_str().map(str::to_string) else {
        eprintln!("SKIP: clipboard holds non-text data; not overwriting it");
        core.stop().await;
        return;
    };
    let probe = "xtraos clip ✓ नमस्ते";
    core.ok("clipboard", json!({"op": "set", "text": probe}))
        .await;
    assert_eq!(
        core.ok("clipboard", json!({"op": "get"})).await["text"],
        probe
    );
    core.ok("clipboard", json!({"op": "set", "text": orig}))
        .await;
    assert_eq!(
        core.ok("clipboard", json!({"op": "get"})).await["text"],
        json!(orig)
    );
    core.stop().await;
}
