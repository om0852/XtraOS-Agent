//! Everyday pack 2 (ADR 0068), interactive: window.snap, files.recent, files.read_text,
//! disk.usage and routine pause/resume on one in-process core with a private XtraOS home.
//! files.recent writes one small file into Downloads (not OneDrive-synced) and deletes it.
mod common;

use common::{code, Core, Testbed};
use serde_json::json;
use xtraos_core::routines;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens and snaps a testbed window; writes one file in Downloads"]
async fn everyday_pack_2() {
    let home = std::env::temp_dir().join(format!("xtraos-everyday-{}", std::process::id()));
    let work = std::env::temp_dir().join(format!("xtraos-everyday-work-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&work).unwrap();
    // SAFETY: set before the core starts and before any other thread reads it.
    unsafe { std::env::set_var("XTRAOS_HOME", &home) };
    let mut core = Core::start().await;
    let mut tb = Testbed::start(300, 200);
    let w = tb.window();
    let skill = |name: &str, args: serde_json::Value| json!({"name": name, "args": args});

    // 1. window.snap: left, then right next to it, then maximize.
    let l = core.ok("run_skill", skill("window.snap", json!({"window": w, "to": "left"}))).await;
    let (lx, lw) = (l["x"].as_i64().unwrap(), l["w"].as_i64().unwrap());
    let now = core.ok("list_windows", json!({"filter": "xtraos testbed"})).await;
    let me = now["windows"].as_array().unwrap().iter().find(|x| x["ref"] == w).unwrap().clone();
    assert!((me["x"].as_i64().unwrap() - lx).abs() <= 3 && (me["w"].as_i64().unwrap() - lw).abs() <= 3, "{me} vs {l}");
    let r = core.ok("run_skill", skill("window.snap", json!({"window": w, "to": "right"}))).await;
    assert_eq!(r["x"].as_i64().unwrap(), lx + lw, "right half starts where the left one ends: {r}");
    core.ok("run_skill", skill("window.snap", json!({"window": w, "to": "maximize"}))).await;
    let now = core.ok("list_windows", json!({"filter": "xtraos testbed"})).await;
    let me = now["windows"].as_array().unwrap().iter().find(|x| x["ref"] == w).unwrap().clone();
    assert_eq!(me["state"], "maximized", "{me}");
    let e = core.call("run_skill", skill("window.snap", json!({"window": w, "to": "diagonal"}))).await;
    assert_eq!(code(&e), "INVALID_ARGS", "{e}");

    // 2. files.recent: a file just written in Downloads is listed first for its pattern.
    let downloads = xtraos_core::everyday::user_folders().into_iter().find(|p| p.ends_with("Downloads")).expect("Downloads");
    let name = format!("xtraos-recent-test-{}.txt", std::process::id());
    let probe = downloads.join(&name);
    std::fs::write(&probe, "probe").unwrap();
    let rec = core.ok("run_skill", skill("files.recent", json!({"days": 1, "pattern": "xtraos-recent-test-*"}))).await;
    let _ = std::fs::remove_file(&probe);
    assert_eq!(rec["files"][0]["path"], probe.display().to_string(), "{rec}");

    // 3. files.read_text: UTF-8, UTF-16 with BOM; binary refused; protected refused.
    std::fs::write(work.join("a.md"), "# Notes\nline two\n").unwrap();
    let mut u16b = vec![0xFF, 0xFE];
    for c in "UTF-16 text".encode_utf16() {
        u16b.extend_from_slice(&c.to_le_bytes());
    }
    std::fs::write(work.join("b.txt"), &u16b).unwrap();
    std::fs::write(work.join("c.bin"), [0x89u8, b'P', b'N', b'G', 0, 0, 7]).unwrap();
    let t = core.ok("run_skill", skill("files.read_text", json!({"path": work.join("a.md").display().to_string()}))).await;
    assert!(t["text"].as_str().unwrap().contains("(2 lines") && t["text"].as_str().unwrap().contains("line two"), "{t}");
    let t = core.ok("run_skill", skill("files.read_text", json!({"path": work.join("b.txt").display().to_string()}))).await;
    assert!(t["text"].as_str().unwrap().ends_with("UTF-16 text"), "{t}");
    let e = core.call("run_skill", skill("files.read_text", json!({"path": work.join("c.bin").display().to_string()}))).await;
    assert_eq!(code(&e), "INVALID_ARGS", "{e}");
    let e = core.call("run_skill", skill("files.read_text", json!({"path": home.join("pairing.token").display().to_string()}))).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "XtraOS's own files: {e}");

    // 4. disk.usage lists C:.
    let d = core.ok("run_skill", skill("disk.usage", json!({}))).await;
    assert!(d["drives"].as_array().unwrap().iter().any(|x| x["drive"] == "C:"), "{d}");
    eprintln!("{}", d["text"]);

    // 5. routine.pause / resume: a paused routine is not run by a due tick.
    let t0 = routines::local_now_ms();
    let r = routines::from_args(&json!({"name": "list-work", "skill": "files.list", "args": {"path": work.display().to_string()}, "every_minutes": 5, "notify": "never"}), t0 - 6 * 60_000, "test").unwrap();
    routines::add_approved(r).unwrap();
    core.ok("run_skill", skill("routine.pause", json!({"name": "list-work"}))).await;
    let l = core.ok("run_skill", skill("routine.list", json!({}))).await;
    assert!(l["text"].as_str().unwrap().contains("(PAUSED)"), "{l}");
    let ran = tokio::task::spawn_blocking(move || routines::run_due(t0)).await.unwrap();
    assert!(ran.is_empty(), "paused: {ran:?}");
    core.ok("run_skill", skill("routine.resume", json!({"name": "list-work"}))).await;
    let ran = tokio::task::spawn_blocking(move || routines::run_due(t0 + 60_000)).await.unwrap();
    assert!(ran.is_empty(), "resumed: no catch-up burst right away: {ran:?}");
    let ran = tokio::task::spawn_blocking(move || routines::run_due(t0 + 6 * 60_000)).await.unwrap();
    assert_eq!(ran, vec!["list-work".to_string()], "runs again on its schedule");

    drop(tb);
    core.stop().await;
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&work);
}
