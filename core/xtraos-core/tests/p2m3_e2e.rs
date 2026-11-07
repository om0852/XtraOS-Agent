//! P2-M3 interactive tests (docs/08 P2-AM-*, P2-SK-*): app map, skills, file skills. They use
//! a private XtraOS data folder (never the user's real app map / skills / policy), open
//! Calculator and the testbed, and create files only under %TEMP%. `#[ignore]`: run via
//! tests/phase2/p2m3-gate.ps1 -Interactive, hands off.
mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::{code, refs_by_name, Core, Testbed};
use serde_json::{json, Value};
use xtraos_core::safety;

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[link(name = "user32")]
extern "system" {
    fn SendMessageW(h: *mut std::ffi::c_void, msg: u32, w: usize, l: isize) -> isize;
    fn GetDlgItem(h: *mut std::ffi::c_void, id: i32) -> *mut std::ffi::c_void;
    fn GetWindowTextW(h: *mut std::ffi::c_void, buf: *mut u16, n: i32) -> i32;
}

fn text(v: &Value) -> String {
    v["text"].as_str().unwrap_or("").to_string()
}

/// A private data folder for this test process (app map, skills, policy).
fn private_home() -> PathBuf {
    let home = std::env::temp_dir().join(format!("xtraos-p2m3-home-{}", std::process::id()));
    std::fs::create_dir_all(home.join("skills")).unwrap();
    std::env::set_var("XTRAOS_HOME", &home);
    xtraos_core::appmap::set_path(home.join("appmap.bin"));
    safety::set_policy_path(home.join("policy.json"));
    home
}

fn work_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("xtraos-sk-{tag}-{}", common::rand_hex(3)));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn s(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// The human presses button `i` in the next core dialog; returns its message.
fn human_answers(i: i32) -> std::thread::JoinHandle<String> {
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
                return String::from_utf16_lossy(&buf[..n.max(0) as usize]);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("no dialog appeared");
    })
}

async fn skill(core: &mut Core, name: &str, args: Value) -> Value {
    core.call("run_skill", json!({"name": name, "args": args}))
        .await
}

// P2-AM-01 + P2-AM-03: scan finds Calculator; a mapped service launches it.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens Calculator"]
async fn am01_am03_scan_and_launch_mapped_service() {
    let _s = SERIAL.lock().await;
    let home = private_home();
    let mut core = Core::start().await;
    let r = core
        .ok("app_map", json!({"op": "scan", "query": "calculator"}))
        .await;
    eprintln!("scan: {}", text(&r));
    let id = text(&r)
        .lines()
        .find(|l| l.contains("WindowsCalculator"))
        .and_then(|l| l.split(" → ").nth(1))
        .expect("Calculator in the Apps folder")
        .to_string();
    let all = core.ok("app_map", json!({"op": "scan"})).await;
    assert!(text(&all).contains("installed apps"), "{all}");
    core.ok(
        "app_map",
        json!({"op": "set", "service": "calc-test", "surface": "desktop", "location": id}),
    )
    .await;
    let w = core
        .ok(
            "launch_app",
            json!({"app": "calc-test", "timeout_ms": 20000}),
        )
        .await;
    eprintln!("launched via app map: {w}");
    assert!(w["title"].as_str().unwrap().contains("Calculator"), "{w}");
    let _ = core.call("close_window", json!({"window": w["ref"]})).await;
    core.stop().await;
    let _ = std::fs::remove_dir_all(home);
}

// P2-AM-02: set/get/list/remove; the file is encrypted; a new core reads it back.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn am02_entries_are_encrypted_and_persist() {
    let _s = SERIAL.lock().await;
    let home = private_home();
    let mut core = Core::start().await;
    core.ok(
        "app_map",
        json!({"op": "set", "service": "Mail", "surface": "browser",
               "location": {"browser": "chrome", "url": "https://mail.example.com", "profile": "Profile 2"}}),
    )
    .await;
    core.stop().await;
    let bytes = std::fs::read(home.join("appmap.bin")).unwrap();
    assert!(
        !String::from_utf8_lossy(&bytes)
            .to_lowercase()
            .contains("mail.example"),
        "app map must not be plain text"
    );
    let mut core = Core::start().await; // "restart"
    let g = core
        .ok("app_map", json!({"op": "get", "service": "mail"}))
        .await;
    assert_eq!(g["location"]["profile"], "Profile 2", "{g}");
    let l = core.ok("app_map", json!({"op": "list"})).await;
    assert!(text(&l).starts_with("mail: browser"), "{l}");
    core.ok("app_map", json!({"op": "remove", "service": "mail"}))
        .await;
    let e = core
        .call("app_map", json!({"op": "get", "service": "mail"}))
        .await;
    assert_eq!(code(&e), "ELEMENT_NOT_FOUND");
    core.stop().await;
    let _ = std::fs::remove_dir_all(home);
}

// P2-SK-01 + P2-SK-02: list, mkdir, copy, move, rename; never overwrite.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn sk01_sk02_file_skills() {
    let _s = SERIAL.lock().await;
    let home = private_home();
    let mut core = Core::start().await;
    let ls = core.ok("list_skills", json!({"app": "files"})).await;
    eprintln!("skills:\n{}", text(&ls));
    assert!(text(&ls).contains("files.copy(from[], to_dir)"));
    let d = work_dir("ops");
    std::fs::write(d.join("a.txt"), "A").unwrap();
    std::fs::write(d.join("b.txt"), "B").unwrap();
    let sub = d.join("Reports");
    let r = skill(&mut core, "files.mkdir", json!({"path": s(&sub)})).await;
    assert!(r.get("result").is_some(), "{r}");
    let r = skill(
        &mut core,
        "files.copy",
        json!({"from": [s(&d.join("a.txt")), s(&d.join("b.txt"))], "to_dir": s(&sub)}),
    )
    .await;
    assert!(r.get("result").is_some(), "{r}");
    assert_eq!(std::fs::read_to_string(sub.join("a.txt")).unwrap(), "A");
    std::fs::write(d.join("a.txt"), "A2").unwrap();
    let r = skill(
        &mut core,
        "files.copy",
        json!({"from": [s(&d.join("a.txt"))], "to_dir": s(&sub)}),
    )
    .await;
    assert_eq!(code(&r), "INVALID_ARGS", "never overwrites: {r}");
    assert_eq!(std::fs::read_to_string(sub.join("a.txt")).unwrap(), "A");
    let moved = d.join("Moved");
    skill(&mut core, "files.mkdir", json!({"path": s(&moved)})).await;
    let r = skill(
        &mut core,
        "files.move",
        json!({"from": [s(&d.join("b.txt"))], "to_dir": s(&moved)}),
    )
    .await;
    assert!(r.get("result").is_some(), "{r}");
    assert!(!d.join("b.txt").exists() && moved.join("b.txt").exists());
    let r = skill(
        &mut core,
        "files.rename",
        json!({"path": s(&moved.join("b.txt")), "new_name": "c.txt"}),
    )
    .await;
    assert!(r.get("result").is_some(), "{r}");
    assert!(moved.join("c.txt").exists());
    let l = skill(
        &mut core,
        "files.list",
        json!({"path": s(&sub), "pattern": "*.txt"}),
    )
    .await;
    let lt = l["result"]["text"].as_str().unwrap().to_string();
    assert!(
        lt.contains("a.txt") && lt.contains("b.txt") && lt.contains("-- 2 items"),
        "{lt}"
    );
    core.stop().await;
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(home);
}

// P2-SK-03: delete asks first; deny keeps the file; allow → Recycle Bin (not gone for good).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn sk03_delete_asks_and_recycles() {
    let _s = SERIAL.lock().await;
    let home = private_home();
    let mut core = Core::start().await;
    let d = work_dir("del");
    let name = format!("xtraos-sk-recycle-{}.txt", common::rand_hex(4));
    let f = d.join(&name);
    std::fs::write(&f, "bye").unwrap();
    let human = human_answers(1); // Deny
    let r = skill(&mut core, "files.delete", json!({"paths": [s(&f)]})).await;
    let msg = human.join().unwrap();
    eprintln!("delete dialog:\n{msg}");
    assert!(msg.contains("Recycle Bin") && msg.contains(&name), "{msg}");
    assert_eq!(code(&r), "BLOCKED_BY_POLICY", "{r}");
    assert!(f.exists(), "denied: file kept");
    let human = human_answers(0); // Allow once
    let r = skill(&mut core, "files.delete", json!({"paths": [s(&f)]})).await;
    human.join().unwrap();
    assert!(r.get("result").is_some(), "{r}");
    assert!(!f.exists(), "moved away");
    // It is in the Recycle Bin; then remove just this test item from there.
    let ps = format!(
        "$i = (New-Object -ComObject Shell.Application).Namespace(10).Items() | Where-Object {{ $_.Name -eq '{name}' }}; \
         if ($i) {{ 'IN_BIN'; foreach ($x in @($i)) {{ $r = $x.Path; $info = Join-Path (Split-Path $r) ('$I' + (Split-Path $r -Leaf).Substring(2)); Remove-Item -LiteralPath $r, $info -Force -ErrorAction SilentlyContinue }} }}"
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &ps])
        .output()
        .unwrap();
    let o = String::from_utf8_lossy(&out.stdout);
    assert!(
        o.contains("IN_BIN"),
        "file not found in the Recycle Bin: {o}"
    );
    core.stop().await;
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(home);
}

// P2-SK-04: protected places are refused before anything happens (no dialog either).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn sk04_protected_paths() {
    let _s = SERIAL.lock().await;
    let home = private_home();
    let mut core = Core::start().await;
    let windir = std::env::var("SystemRoot").unwrap();
    let d = work_dir("prot");
    std::fs::write(home.join("policy.json"), "{}").unwrap();
    for (name, args) in [
        ("files.list", json!({"path": windir})),
        ("files.list", json!({"path": "C:\\"})),
        (
            "files.delete",
            json!({"paths": [s(&home.join("policy.json"))]}),
        ),
        (
            "files.copy",
            json!({"from": [s(&d)], "to_dir": "\\\\server\\share"}),
        ),
        (
            "files.rename",
            json!({"path": s(&home.join("policy.json")), "new_name": "x.json"}),
        ),
    ] {
        let r = skill(&mut core, name, args.clone()).await;
        assert_eq!(code(&r), "BLOCKED_BY_POLICY", "{name} {args}: {r}");
    }
    assert!(
        safety::prompt_windows().is_empty(),
        "no dialog for protected paths"
    );
    assert!(home.join("policy.json").exists());
    core.stop().await;
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(home);
}

// P2-SK-05 + P2-SK-06: a macro skill runs as a batch (policy per step); a routine file task
// (new folder + copy + open) in ONE call.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens Explorer"]
async fn sk05_sk06_macro_skills() {
    let _s = SERIAL.lock().await;
    let home = private_home();
    std::fs::write(
        home.join("skills").join("testbed.json"),
        r#"{"name": "testbed.note_and_send", "description": "write a note, press Send",
            "args": {"field": {}, "send": {}, "note": {"default": "hello"}},
            "steps": [{"tool": "set_value", "args": {"ref": "${field}", "value": "${note}"}},
                      {"tool": "click", "args": {"ref": "${send}"}}]}"#,
    )
    .unwrap();
    std::fs::write(
        home.join("skills").join("reports.json"),
        r#"{"name": "reports.file", "description": "put files into a new folder and show it",
            "args": {"files": {}, "folder": {}},
            "steps": [{"tool": "run_skill", "args": {"name": "files.mkdir", "args": {"path": "${folder}"}}},
                      {"tool": "run_skill", "args": {"name": "files.copy", "args": {"from": "${files}", "to_dir": "${folder}"}}},
                      {"tool": "run_skill", "args": {"name": "files.open", "args": {"path": "${folder}"}}}]}"#,
    )
    .unwrap();
    let mut core = Core::start().await;
    let ls = core.ok("list_skills", json!({})).await;
    assert!(text(&ls).contains("testbed.note_and_send(") && text(&ls).contains("[macro]"));

    // P2-SK-05: the Send step inside the macro still needs the user's OK.
    let mut tb = Testbed::start(120, 120);
    let o = core
        .ok("observe", json!({"window": tb.window(), "full": true}))
        .await;
    let refs = refs_by_name(&text(&o));
    let human = human_answers(1);
    let r = skill(
        &mut core,
        "testbed.note_and_send",
        json!({"field": refs["text"], "send": refs["send"], "note": "from a macro"}),
    )
    .await;
    human.join().unwrap();
    eprintln!("macro: {}", r["result"]["steps"]);
    assert_eq!(r["result"]["steps"][0]["ok"], true);
    assert_eq!(
        r["result"]["steps"][1]["error"]["code"],
        "BLOCKED_BY_POLICY"
    );
    let st = tb.state();
    assert_eq!(st["text"], "from a macro");
    assert_eq!(st["clicks"]["send"], 0);
    drop(tb);

    // P2-SK-06: one call for a routine file task.
    let d = work_dir("report");
    std::fs::write(d.join("q3.pdf"), "pdf").unwrap();
    let folder = d.join("Q3 Reports");
    let r = skill(
        &mut core,
        "reports.file",
        json!({"files": [s(&d.join("q3.pdf"))], "folder": s(&folder)}),
    )
    .await;
    eprintln!("routine file task: {}", r["result"]["steps"]);
    assert_eq!(r["result"]["ok"], true, "{r}");
    assert!(folder.join("q3.pdf").exists());
    let win = r["result"]["steps"][2]["result"]["ref"].clone();
    if win.is_string() {
        let _ = core.call("close_window", json!({"window": win})).await;
    }
    // A macro may not run another macro.
    std::fs::write(
        home.join("skills").join("nest.json"),
        r#"{"name": "nest", "args": {}, "steps": [{"tool": "run_skill", "args": {"name": "reports.file"}}]}"#,
    )
    .unwrap();
    let r = skill(&mut core, "nest", json!({})).await;
    assert_eq!(
        r["result"]["steps"][0]["error"]["code"], "INVALID_ARGS",
        "{r}"
    );
    core.stop().await;
    let _ = std::fs::remove_dir_all(&d);
    let _ = std::fs::remove_dir_all(home);
}
