//! P2-M6 browser bridge tests (docs/08 P2-BR-*, ADR 0032). A fake XtraDevPilot bridge (raw
//! TCP + the core's own WebSocket codec) runs in this process: no browser, no human.
//! The dialog / kill-switch tests are `#[ignore]` (they show a core dialog or flip the kill
//! switch): run via tests/phase2/p2m6-gate.ps1 -Automated. The live test against the real
//! bridge runs only with XTRAOS_TEST_BRIDGE=1 (read only).
mod common;

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{code, Core};
use serde_json::{json, Value};
use xtraos_core::browser::{self, ws};
use xtraos_core::safety;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const GOOD: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[link(name = "user32")]
extern "system" {
    fn SendMessageW(h: *mut std::ffi::c_void, msg: u32, w: usize, l: isize) -> isize;
    fn PostMessageW(h: *mut std::ffi::c_void, msg: u32, w: usize, l: isize) -> i32;
    fn GetDlgItem(h: *mut std::ffi::c_void, id: i32) -> *mut std::ffi::c_void;
    fn GetWindowTextW(h: *mut std::ffi::c_void, buf: *mut u16, n: i32) -> i32;
}

struct Fake {
    port: u16,
    log: Arc<Mutex<Vec<Value>>>,
    tab: Arc<Mutex<Value>>,
}

impl Fake {
    fn actions(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .map(|v| v["action"].as_str().unwrap_or("").to_string())
            .collect()
    }
}

/// A fake bridge: WebSocket handshake, AUTH with `GOOD`, scripted replies.
fn fake_bridge() -> Fake {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = l.local_addr().unwrap().port();
    let log = Arc::new(Mutex::new(Vec::new()));
    let tab = Arc::new(Mutex::new(
        json!({"id": 7, "title": "Example", "url": "https://example.com/", "status": "complete"}),
    ));
    let (log2, tab2) = (log.clone(), tab.clone());
    std::thread::spawn(move || {
        for s in l.incoming().flatten() {
            let (log, tab) = (log2.clone(), tab2.clone());
            std::thread::spawn(move || serve(s, log, tab));
        }
    });
    Fake { port, log, tab }
}

fn serve(s: std::net::TcpStream, log: Arc<Mutex<Vec<Value>>>, tab: Arc<Mutex<Value>>) {
    let mut w = s.try_clone().unwrap();
    let mut r = BufReader::new(s);
    let mut key = String::new();
    loop {
        let mut line = String::new();
        if r.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        if let Some(v) = line.strip_prefix("Sec-WebSocket-Key:") {
            key = v.trim().to_string();
        }
        if line == "\r\n" {
            break;
        }
    }
    let _ = write!(
        w,
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        ws::accept_key(&key)
    );
    let send = |w: &mut std::net::TcpStream, v: Value| {
        let _ = w.write_all(&ws::encode_frame(
            ws::OP_TEXT,
            v.to_string().as_bytes(),
            None,
        ));
    };
    let mut authed = false;
    loop {
        let Ok(Some((_, d))) = ws::read_message(&mut r, &mut |_| Ok(())) else {
            return;
        };
        let m: Value = serde_json::from_slice(&d).unwrap_or(json!({}));
        if m["type"] == "AUTH" {
            authed = m["token"] == GOOD;
            send(&mut w, json!({"type": "AUTH_RESULT", "ok": authed}));
            if !authed {
                return;
            }
            continue;
        }
        let id = m["id"].clone();
        if !authed {
            send(&mut w, json!({"id": id, "error": "UNAUTHORIZED: token"}));
            continue;
        }
        log.lock().unwrap().push(m.clone());
        let reply = match m["action"].as_str().unwrap_or("") {
            "LIST_BROWSERS" => {
                json!({"id": id, "result": [{"browserId": 1, "active": true, "title": "Fake", "url": "https://example.com/"}]})
            }
            "GET_TAB_INFO" => json!({"id": id, "result": tab.lock().unwrap().clone()}),
            "GET_CLEAN_DOM" => {
                json!({"id": id, "result": format!("<div>{}</div>", "x".repeat(70_000))})
            }
            "CLICK_ELEMENT" if m["selector"] == "#missing" => {
                json!({"id": id, "error": "Element not found: #missing"})
            }
            _ => json!({"id": id, "result": {"success": true}}),
        };
        send(&mut w, reply);
    }
}

fn use_fake(f: &Fake, token: &str) {
    browser::disconnect();
    std::env::set_var("XTRAOS_BROWSER_PORT", f.port.to_string());
    std::env::set_var("XTRADEVPILOT_TOKEN", token);
}

fn private_policy() {
    let home = std::env::temp_dir().join(format!("xtraos-p2m6-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    safety::set_policy_path(home.join("policy.json"));
}

fn text(v: &Value) -> String {
    v["result"]["text"].as_str().unwrap_or("").to_string()
}

async fn br(core: &mut Core, args: Value) -> Value {
    core.call("browser", args).await
}

// P2-BR-01: auth (bad token refused, good token works), actions map to the wire names.
#[tokio::test(flavor = "multi_thread")]
async fn br01_auth_and_mapping() {
    let _s = SERIAL.lock().await;
    private_policy();
    let f = fake_bridge();
    let mut core = Core::start().await;
    use_fake(&f, &"f".repeat(64));
    let r = br(&mut core, json!({"action": "list_browsers"})).await;
    assert_eq!(code(&r), "NEEDS_HUMAN", "bad token: {r}");
    assert!(f.actions().is_empty(), "nothing relayed without auth");

    use_fake(&f, GOOD);
    let r = br(&mut core, json!({"action": "list_browsers"})).await;
    assert!(text(&r).starts_with("1 browser(s)"), "{r}");
    let r = br(&mut core, json!({"action": "tab_info"})).await;
    assert_eq!(
        text(&r),
        "tab 7 \"Example\" https://example.com/ (complete)"
    );
    let r = br(&mut core, json!({"action": "click", "text": "Docs"})).await;
    assert!(text(&r).starts_with("ok browser click"), "{r}");
    let clicks: Vec<Value> = f
        .log
        .lock()
        .unwrap()
        .iter()
        .filter(|m| m["action"] == "CLICK_ELEMENT")
        .cloned()
        .collect();
    assert_eq!(clicks[0]["selector"], "text=Docs");
    let r = br(
        &mut core,
        json!({"action": "click", "selector": "#missing"}),
    )
    .await;
    assert_eq!(code(&r), "ELEMENT_NOT_FOUND", "{r}");
    let r = br(&mut core, json!({"action": "dom"})).await;
    assert!(
        text(&r).contains("cut at 60000 characters"),
        "long DOM is cut"
    );
    eprintln!("BR-01 ok: wire actions {:?}", f.actions());
    browser::disconnect();
    core.stop().await;
}

// P2-BR-02: refused actions never reach the bridge; password fields and buy buttons are denied.
#[tokio::test(flavor = "multi_thread")]
async fn br02_refusals_never_reach_the_bridge() {
    let _s = SERIAL.lock().await;
    private_policy();
    let f = fake_bridge();
    let mut core = Core::start().await;
    use_fake(&f, GOOD);
    for (args, want) in [
        (
            json!({"action": "execute_script", "script": "document.cookie"}),
            "BLOCKED_BY_POLICY",
        ),
        (json!({"action": "manage_cookies"}), "BLOCKED_BY_POLICY"),
        (
            json!({"action": "navigate", "url": "javascript:alert(1)"}),
            "INVALID_ARGS",
        ),
        (
            json!({"action": "type", "selector": "input[type=password]", "text": "hunter2"}),
            "BLOCKED_BY_POLICY",
        ),
        (
            json!({"action": "click", "text": "Buy now"}),
            "BLOCKED_BY_POLICY",
        ),
    ] {
        let r = br(&mut core, args.clone()).await;
        assert_eq!(code(&r), want, "{args} → {r}");
    }
    let sent = f.actions();
    assert!(
        !sent.iter().any(|a| [
            "EXECUTE_SCRIPT",
            "MANAGE_COOKIES",
            "NAVIGATE",
            "TYPE_TEXT",
            "CLICK_ELEMENT"
        ]
        .contains(&a.as_str())),
        "only GET_TAB_INFO lookups may have been sent: {sent:?}"
    );
    eprintln!("BR-02 ok: refused; bridge saw only {sent:?}");
    browser::disconnect();
    core.stop().await;
}

// P2-BR-03: no bridge running → NEEDS_HUMAN with a hint.
#[tokio::test(flavor = "multi_thread")]
async fn br03_bridge_down_needs_human() {
    let _s = SERIAL.lock().await;
    private_policy();
    let mut core = Core::start().await;
    let free = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    browser::disconnect();
    std::env::set_var("XTRAOS_BROWSER_PORT", free.to_string());
    std::env::set_var("XTRADEVPILOT_TOKEN", GOOD);
    let r = br(&mut core, json!({"action": "list_browsers"})).await;
    assert_eq!(code(&r), "NEEDS_HUMAN", "{r}");
    assert!(
        r["error"]["hint"]
            .as_str()
            .unwrap_or("")
            .contains("XtraDevPilot"),
        "{r}"
    );
    eprintln!("BR-03 ok: {}", r["error"]["message"]);
    core.stop().await;
}

fn human_answers(i: i32) -> std::thread::JoinHandle<Option<String>> {
    std::thread::spawn(move || {
        let t0 = Instant::now();
        while t0.elapsed() < Duration::from_secs(10) {
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

// P2-BR-04: Enter / a send click on a messaging site asks; Deny → nothing reaches the page.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "shows a core approval dialog (answered by the test)"]
async fn br04_send_on_messaging_site_asks() {
    let _s = SERIAL.lock().await;
    private_policy();
    let f = fake_bridge();
    *f.tab.lock().unwrap() = json!({"id": 3, "title": "WhatsApp", "url": "https://web.whatsapp.com/", "status": "complete"});
    let mut core = Core::start().await;
    use_fake(&f, GOOD);
    let human = human_answers(1); // Deny
    let r = br(&mut core, json!({"action": "press_key", "key": "Enter"})).await;
    let msg = human.join().unwrap().expect("a dialog");
    eprintln!("dialog:\n{msg}");
    assert!(
        msg.contains("web.whatsapp.com") && msg.contains("message_send"),
        "{msg}"
    );
    assert_eq!(code(&r), "BLOCKED_BY_POLICY", "{r}");
    assert!(
        !f.actions().contains(&"PRESS_KEY".to_string()),
        "denied: not sent"
    );
    let human = human_answers(0); // Allow once
    let r = br(
        &mut core,
        json!({"action": "click", "selector": "button[aria-label='Send']"}),
    )
    .await;
    human.join().unwrap().expect("a dialog for the send click");
    assert!(r.get("result").is_some(), "{r}");
    assert!(
        f.actions().contains(&"CLICK_ELEMENT".to_string()),
        "allowed: sent"
    );
    eprintln!("BR-04 ok: messaging send asks; Deny blocks, Allow sends");
    browser::disconnect();
    core.stop().await;
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

// P2-BR-05: after the kill switch, browser actions are refused.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "flips the kill switch"]
async fn br05_kill_switch_stops_browser_actions() {
    let _s = SERIAL.lock().await;
    private_policy();
    let f = fake_bridge();
    let mut core = Core::start().await;
    use_fake(&f, GOOD);
    safety::stop();
    let r = br(
        &mut core,
        json!({"action": "navigate", "url": "https://example.com"}),
    )
    .await;
    resume_from_tray();
    assert_eq!(code(&r), "STOPPED", "{r}");
    assert!(!f.actions().contains(&"NAVIGATE".to_string()));
    eprintln!("BR-05 ok: STOPPED, nothing sent");
    browser::disconnect();
    core.stop().await;
}

// P2-BR-LIVE: the real XtraDevPilot bridge (read only). Prints counts, never URLs.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs the running XtraDevPilot bridge"]
async fn br_live_list_browsers() {
    if std::env::var("XTRAOS_TEST_BRIDGE").ok().as_deref() != Some("1") {
        eprintln!("SKIP: set XTRAOS_TEST_BRIDGE=1 with XtraDevPilot running");
        return;
    }
    let _s = SERIAL.lock().await;
    private_policy();
    browser::disconnect();
    std::env::remove_var("XTRAOS_BROWSER_PORT");
    std::env::remove_var("XTRADEVPILOT_TOKEN");
    let mut core = Core::start().await;
    let r = br(&mut core, json!({"action": "list_browsers"})).await;
    let t = text(&r);
    eprintln!(
        "BR-LIVE: {}",
        if t.is_empty() {
            format!("error {}", code(&r))
        } else {
            t.lines().next().unwrap_or("").to_string()
        }
    );
    assert!(t.contains("browser(s)"), "{}", code(&r));
    browser::disconnect();
    core.stop().await;
}
