//! P6-M6 work beside the user (ADR 0049), interactive and opt-in (XTRAOS_TEST_BROWSERS=1):
//! a web form in Edge/Chrome (throw-away private profiles) sits behind the TopMost testbed,
//! which plays the user's window. The page's own scripts write what they noticed into the
//! window title, so every check is on what the PAGE saw, not on what UI Automation says.
//! Keep hands off while it runs: borrowing waits for the user to be idle.
mod common;

use std::time::{Duration, Instant};

use common::{Core, Testbed};
use serde_json::{json, Value};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const PAGE: &str = r#"<!doctype html><meta charset="utf-8"><title>TITLE</title>
<label>Full name <input id="name" aria-label="Full name"></label>
<label>Country <select id="country" aria-label="Country"><option>India</option><option>Japan</option><option>Peru</option></select></label>
<div id="ce" contenteditable="true" aria-label="Notes" role="textbox" style="border:1px solid #888;min-height:20px"></div>
<span id="plain" style="border:1px dashed #888;padding:3px">Plain div click</span>
<script>
const S = {n:'', c:'India', cc:0, ce:'', cei:0, pl:0};
const show = () => { document.title = 'TITLE ' + JSON.stringify(S); };
const $ = (id) => document.getElementById(id);
$('name').addEventListener('input', e => { S.n = e.target.value; show(); });
$('country').addEventListener('change', e => { S.cc++; S.c = e.target.value; show(); });
$('ce').addEventListener('input', () => { S.cei++; S.ce = $('ce').textContent; show(); });
$('plain').addEventListener('click', () => { S.pl++; show(); });
show();
</script>"#;

struct EndBrowserOnDrop(String, &'static str);

impl Drop for EndBrowserOnDrop {
    fn drop(&mut self) {
        let ps = format!(
            "Get-CimInstance Win32_Process -Filter \"Name='{1}'\" | Where-Object {{ $_.CommandLine -like '*xtraos-p6m6-{0}*' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}",
            self.0, self.1
        );
        let _ = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &ps])
            .status();
        std::thread::sleep(Duration::from_secs(1));
        let dir = std::env::temp_dir().join(format!("xtraos-p6m6-{}", self.0));
        let _ = std::process::Command::new("cmd")
            .args(["/c", "rd", "/s", "/q"])
            .arg(&dir)
            .status();
    }
}

fn browsers() -> Vec<(std::path::PathBuf, &'static str, &'static str)> {
    let mut v = Vec::new();
    let edge = std::path::Path::new(&std::env::var("ProgramFiles(x86)").unwrap_or_default())
        .join(r"Microsoft\Edge\Application\msedge.exe");
    if edge.exists() {
        v.push((edge, "msedge.exe", "--inprivate"));
    }
    let chrome = std::path::Path::new(&std::env::var("ProgramFiles").unwrap_or_default())
        .join(r"Google\Chrome\Application\chrome.exe");
    if chrome.exists() {
        v.push((chrome, "chrome.exe", "--incognito"));
    }
    v
}

/// The ref of the first `e12 Role "Name"` line.
fn ref_of(text: &str, role: &str, name: &str) -> Option<String> {
    let want = format!(" {role} \"{name}\"");
    text.lines()
        .find(|l| l.contains(&want))
        .and_then(|l| l.split_whitespace().next())
        .map(String::from)
}

async fn page_state(core: &mut Core, title: &str) -> Value {
    let l = core.ok("list_windows", json!({"filter": title})).await;
    let t = l["windows"][0]["title"].as_str().unwrap_or("").to_string();
    let json_part = t.find('{').map(|i| &t[i..]).unwrap_or("{}");
    let end = json_part.rfind('}').map(|i| &json_part[..=i]).unwrap_or("{}");
    serde_json::from_str(end).unwrap_or(Value::Null)
}

async fn focused(core: &mut Core, w: &str) -> bool {
    let l = core.ok("list_windows", json!({})).await;
    l["windows"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["ref"] == w && x["focused"] == true)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive + browsers: a web form behind the testbed; hands off"]
async fn p6m6_form_behind_the_users_window() {
    if std::env::var("XTRAOS_TEST_BROWSERS").ok().as_deref() != Some("1") {
        eprintln!("skipped: set XTRAOS_TEST_BROWSERS=1");
        return;
    }
    let _s = SERIAL.lock().await;
    let list = browsers();
    assert!(!list.is_empty(), "no Edge or Chrome installed");
    let mut core = Core::start().await;
    for (exe, image, private) in &list {
        let tag = common::rand_hex(6);
        let _end = EndBrowserOnDrop(tag.clone(), image);
        let dir = std::env::temp_dir().join(format!("xtraos-p6m6-{tag}"));
        std::fs::create_dir_all(&dir).unwrap();
        let title = format!("FORM{tag}");
        let page = dir.join("form.html");
        std::fs::write(&page, PAGE.replace("TITLE", &title)).unwrap();
        let url = format!("file:///{}", page.display().to_string().replace('\\', "/"));
        std::process::Command::new(exe)
            .args([
                &format!("--user-data-dir={}", dir.join("profile").display()),
                *private,
                "--no-first-run",
                "--new-window",
                "--window-position=200,160",
                "--window-size=900,700",
                &url,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let t0 = Instant::now();
        let w = loop {
            let r = core.call("window_bounds", json!({"window": title})).await;
            if let Some(w) = r["result"]["ref"].as_str() {
                break w.to_string();
            }
            assert!(t0.elapsed() < Duration::from_secs(20), "{image}: {r}");
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        let o = core.ok("observe", json!({"window": w, "full": true})).await;
        let text = o["text"].as_str().unwrap().to_string();
        let country = ref_of(&text, "ComboBox", "Country").expect("Country");
        let name = ref_of(&text, "Edit", "Full name").expect("Full name");
        let notes = ref_of(&text, "Edit", "Notes").expect("Notes");
        let plain = ref_of(&text, "Text", "Plain div click").expect("plain div");

        // The user's window comes up in front of the browser.
        let tb = Testbed::start(200, 160);
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(focused(&mut core, &tb.window()).await, "testbed in front");

        // 1. <select>: the option is selected and the page gets its change event.
        let r = core.ok("set_value", json!({"ref": country, "value": "japan"})).await;
        assert_eq!(r["via"], "select_option", "{image}: {r}");
        assert_eq!(r["option"], "Japan", "{r}");
        let e = core.call("set_value", json!({"ref": country, "value": "Chile"})).await;
        assert_eq!(common::code(&e), "ELEMENT_NOT_FOUND", "{e}");
        assert!(e.to_string().contains("Peru"), "options are listed: {e}");

        // 2. A browser text field says that rich-text editors may not notice.
        let r = core.ok("set_value", json!({"ref": name, "value": "Om Test"})).await;
        assert!(r["note"].as_str().is_some_and(|n| n.contains("type")), "{r}");

        // 3. A plain clickable <div> (seen as its Text) is clicked in the background.
        let r = core.ok("click", json!({"ref": plain})).await;
        assert_eq!(r["mode"], "background", "{image}: {r}");
        assert!(r["via"].as_str().unwrap().starts_with("parent"), "{r}");

        // 4. borrow:false keeps the old refusal.
        let e = core
            .call("type", json!({"ref": notes, "text": "x", "borrow": false}))
            .await;
        assert_eq!(common::code(&e), "NEEDS_HUMAN", "{e}");

        // 5. type {ref} into the rich-text field: borrowed, then the testbed is in front again.
        let r = core.ok("type", json!({"ref": notes, "text": "hello"})).await;
        assert!(r["borrowed_ms"].as_u64().is_some(), "{image}: {r}");
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(focused(&mut core, &tb.window()).await, "{image}: the user's window is back in front");

        let s = page_state(&mut core, &title).await;
        eprintln!("{image}: page saw {s}; borrowed {} ms", r["borrowed_ms"]);
        assert_eq!(s["c"], "Japan", "{image}: {s}");
        assert!(s["cc"].as_u64().unwrap_or(0) >= 1, "{s}");
        assert_eq!(s["n"], "Om Test", "{s}");
        assert_eq!(s["pl"], 1, "{s}");
        assert_eq!(s["ce"], "hello", "{s}");
        assert!(s["cei"].as_u64().unwrap_or(0) >= 1, "input events: {s}");
        drop(tb);
        // Not close_window: the browser was not opened by the agent, so that asks the user
        // (P2-M4). EndBrowserOnDrop ends this test's own browser instead.
        let _ = &w;
    }
    core.stop().await;
}
