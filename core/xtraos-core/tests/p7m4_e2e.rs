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
