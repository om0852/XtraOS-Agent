//! P7-M4 routines (ADR 0063), interactive: an in-process core with a private XtraOS home.
//! The "add" dialog needs a real click by the user, so the test stores the routine through the
//! function `routine.add` calls after the user approves; everything after that is the real path.
mod common;

use common::{code, Core};
use serde_json::json;
use xtraos_core::{agents, routines};

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: starts the safety UI; takes ~3 s"]
async fn p7m4_routines() {
    let home = std::env::temp_dir().join(format!("xtraos-routines-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let work_dir = std::env::temp_dir().join(format!("xtraos-routines-work-{}", std::process::id()));
    std::fs::create_dir_all(&work_dir).unwrap();
    std::fs::write(work_dir.join("a.txt"), "x").unwrap();
    // SAFETY: set before the core starts and before any other thread reads it.
    unsafe { std::env::set_var("XTRAOS_HOME", &home) };
    let mut base = Core::start().await;
    let work = work_dir.display().to_string();

    // A routine added 6 minutes ago, every 5 minutes: due now.
    let now = routines::local_now_ms();
    let r = routines::from_args(&json!({"name": "List-Work", "skill": "files.list", "args": {"path": work}, "every_minutes": 5}), now - 6 * 60_000, "test").unwrap();
    routines::add_approved(r).unwrap();
    assert!(routines::add_approved(routines::from_args(&json!({"name": "list-work", "skill": "files.list", "args": {"path": work}, "every_minutes": 5}), now, "t").unwrap()).is_err(), "names are unique");

    // The scheduler tick runs it once, as its own agent.
    let ran = tokio::task::spawn_blocking(|| routines::run_due(routines::local_now_ms())).await.unwrap();
    assert_eq!(ran, vec!["list-work".to_string()]);
    let again = tokio::task::spawn_blocking(|| routines::run_due(routines::local_now_ms())).await.unwrap();
    assert!(again.is_empty(), "not due again for 5 minutes");
    let s = base.ok("status", json!({})).await;
    assert!(s["agents"].as_array().unwrap().iter().any(|a| a["client"] == "routine:list-work" && a["last_action"] == "run_skill"), "{s}");

    let l = base.ok("run_skill", json!({"name": "routine.list"})).await;
    assert!(l["text"].as_str().unwrap().contains("list-work: files.list every 5 minutes — last run 20"), "{l}");

    // Run now through run_skill; then paused by the user: STOPPED, recorded as a failed run.
    let r = base.ok("run_skill", json!({"name": "routine.run", "args": {"name": "list-work"}})).await;
    assert!(r["text"].as_str().unwrap().contains("a.txt"), "{r}");
    agents::set_paused("routine:list-work", true);
    let e = base.call("run_skill", json!({"name": "routine.run", "args": {"name": "list-work"}})).await;
    assert_eq!(code(&e), "STOPPED", "{e}");
    agents::set_paused("routine:list-work", false);
    let sum = base.ok("run_skill", json!({"name": "routine.summary"})).await;
    assert!(sum["text"].as_str().unwrap().contains("list-work: 3 run(s), 1 failed"), "{sum}");

    // Checked before the dialog: protected paths and credential-looking arguments.
    let e = base.call("run_skill", json!({"name": "routine.add", "args": {"name": "sys", "skill": "files.list", "args": {"path": "C:/Windows/System32"}, "every_minutes": 30}})).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "{e}");
    let e = base.call("run_skill", json!({"name": "routine.add", "args": {"name": "loop", "skill": "routine.run", "args": {"name": "x"}, "every_minutes": 30}})).await;
    assert_eq!(code(&e), "INVALID_ARGS", "{e}");

    // A file changed outside XtraOS is not trusted (its routines wait for the user).
    let f = home.join("routines.json");
    let text = std::fs::read_to_string(&f).unwrap().replace("every_minutes\": 5", "every_minutes\": 6");
    std::fs::write(&f, text).unwrap();
    routines::reload();
    let l = base.ok("run_skill", json!({"name": "routine.list"})).await;
    assert_eq!(l["trusted"], false, "{l}");
    assert!(l["text"].as_str().unwrap().starts_with("PAUSED"), "{l}");

    // Removing needs no dialog, and re-trusts the file the core writes.
    base.ok("run_skill", json!({"name": "routine.remove", "args": {"name": "list-work"}})).await;
    let l = base.ok("run_skill", json!({"name": "routine.list"})).await;
    assert_eq!(l["trusted"], true, "{l}");
    assert_eq!(l["routines"].as_array().unwrap().len(), 0, "{l}");
    base.stop().await;
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&work_dir);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: starts the safety UI; takes ~15 s (files must settle 10 s)"]
async fn p7m4_new_file_trigger() {
    let home = std::env::temp_dir().join(format!("xtraos-routines-nf-{}", std::process::id()));
    let inbox = std::env::temp_dir().join(format!("xtraos-routines-inbox-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&inbox);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&inbox).unwrap();
    std::fs::write(inbox.join("already-here.txt"), "old").unwrap();
    // SAFETY: set before the core starts and before any other thread reads it.
    unsafe { std::env::set_var("XTRAOS_HOME", &home) };
    let mut base = Core::start().await;
    std::thread::sleep(std::time::Duration::from_millis(50));

    // For each new *.txt: files.find in the inbox for exactly that name ({name} substituted).
    let r = routines::from_args(
        &json!({"name": "inbox", "skill": "files.find", "args": {"path": inbox.display().to_string(), "pattern": "{name}"},
                "new_file_in": inbox.display().to_string(), "pattern": "*.txt", "notify": "never"}),
        routines::local_now_ms(),
        "test",
    )
    .unwrap();
    routines::add_approved(r).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(inbox.join("new-report.txt"), "new").unwrap();
    std::fs::write(inbox.join("big.txt.crdownload"), "partial").unwrap();
    std::fs::write(inbox.join("photo.jpg"), "jpg").unwrap();

    // Not settled yet (written < 10 s ago): nothing runs.
    let ran = tokio::task::spawn_blocking(|| routines::run_due(routines::local_now_ms())).await.unwrap();
    assert!(ran.is_empty(), "{ran:?}");
    tokio::time::sleep(std::time::Duration::from_secs(11)).await;
    let ran = tokio::task::spawn_blocking(|| routines::run_due(routines::local_now_ms())).await.unwrap();
    assert_eq!(ran, vec!["inbox".to_string()], "only new-report.txt: not the old file, the download or the jpg");
    let again = tokio::task::spawn_blocking(|| routines::run_due(routines::local_now_ms())).await.unwrap();
    assert!(again.is_empty(), "each file once");

    let sum = base.ok("run_skill", json!({"name": "routine.summary"})).await;
    let t = sum["text"].as_str().unwrap();
    assert!(t.contains("inbox: 1 run(s), 0 failed") && t.contains("new-report.txt"), "{t}");
    assert!(!t.contains("already-here"), "{t}");
    let l = base.ok("run_skill", json!({"name": "routine.list"})).await;
    assert!(l["text"].as_str().unwrap().contains("for each new *.txt file in"), "{l}");
    base.ok("run_skill", json!({"name": "routine.remove", "args": {"name": "inbox"}})).await;
    base.stop().await;
    let _ = std::fs::remove_dir_all(&home);
    let _ = std::fs::remove_dir_all(&inbox);
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: shows one Windows notification"]
async fn p7m4_one_time_reminder() {
    let home = std::env::temp_dir().join(format!("xtraos-routines-once-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: set before the core starts and before any other thread reads it.
    unsafe { std::env::set_var("XTRAOS_HOME", &home) };
    let mut base = Core::start().await;

    let now = routines::local_now_ms();
    let at = routines::show_time(now + 2 * 60_000);
    let r = routines::from_args(
        &json!({"name": "call-bank", "skill": "notify.show", "args": {"text": "XtraOS test reminder: call the bank"}, "once_at": at, "notify": "never"}),
        now,
        "test",
    )
    .unwrap();
    routines::add_approved(r).unwrap();
    let l = base.ok("run_skill", json!({"name": "routine.list"})).await;
    assert!(l["text"].as_str().unwrap().contains(&format!("call-bank: notify.show once at {at}")), "{l}");

    // Not yet; then at its time (the tick's clock is passed in) it runs once and is removed.
    let ran = tokio::task::spawn_blocking(move || routines::run_due(now + 30_000)).await.unwrap();
    assert!(ran.is_empty(), "{ran:?}");
    let ran = tokio::task::spawn_blocking(move || routines::run_due(now + 3 * 60_000)).await.unwrap();
    assert_eq!(ran, vec!["call-bank".to_string()]);
    let l = base.ok("run_skill", json!({"name": "routine.list"})).await;
    assert_eq!(l["routines"].as_array().unwrap().len(), 0, "removed after its run: {l}");
    let sum = base.ok("run_skill", json!({"name": "routine.summary"})).await;
    let t = sum["text"].as_str().unwrap();
    assert!(t.contains("call-bank: 1 run(s), 0 failed") && t.contains("notification shown"), "{t}");

    // A past time is refused when adding.
    let e = base.call("run_skill", json!({"name": "routine.add", "args": {"name": "late", "skill": "notify.show", "args": {"text": "x"}, "once_at": "2020-01-01 09:00"}})).await;
    assert_eq!(common::code(&e), "INVALID_ARGS", "{e}");
    base.stop().await;
    let _ = std::fs::remove_dir_all(&home);
}
