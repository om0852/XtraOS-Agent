//! Team policies (ADR 0073), interactive: a real core with an organisation key and a signed team
//! policy in a private team folder; then the file is tampered with (fail closed) and removed.
mod common;

use common::{code, Core, Testbed};
use serde_json::json;
use xtraos_core::team;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: starts the safety UI, opens the testbed"]
async fn team_policy_is_a_floor_and_fails_closed() {
    let base = std::env::temp_dir().join(format!("xtraos-team-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let (home, teamd, admin, work) = (base.join("home"), base.join("team"), base.join("admin"), base.join("work"));
    for d in [&home, &teamd.join("team-keys"), &admin, &work] {
        std::fs::create_dir_all(d).unwrap();
    }
    // SAFETY: set before the core starts and before any other thread reads them.
    unsafe {
        std::env::set_var("XTRAOS_HOME", &home);
        std::env::set_var("XTRAOS_TEAM_DIR", &teamd);
    }

    // The admin makes a key, trusts its public half on this PC and signs a policy.
    let msg = team::keygen(&admin).unwrap();
    let kid = msg.lines().next().unwrap().trim_start_matches("key ").to_string();
    std::fs::copy(admin.join(format!("{kid}.pub")), teamd.join("team-keys").join(format!("{kid}.pub"))).unwrap();
    std::fs::write(admin.join("policy.json"), r#"{"classes": {"file_change": "deny"}, "clients": {"agent-team-ro": "read_only"}}"#).unwrap();
    team::sign_file(&admin.join("policy.json"), &admin.join(format!("{kid}.key")), &teamd.join("team-policy.json")).unwrap();

    let mut tb = Testbed::start(240, 200);
    let mut core = Core::start().await;
    let mut ro = core.another("agent-team-ro").await;
    let w = tb.window();
    let folder = |n: &str| json!({"name": "files.mkdir", "args": {"path": work.join(n).display().to_string()}});

    // status shows it; a team deny beats the user's allow (file_change is allowed by default).
    let s = core.ok("status", json!({})).await;
    assert_eq!(s["team"]["active"], true, "{s}");
    assert_eq!(s["team"]["key_id"], kid.as_str(), "{s}");
    let e = core.call("run_skill", folder("a")).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "{e}");
    assert!(e.to_string().contains("organisation rule"), "{e}");
    assert!(!work.join("a").exists());

    // The team's client profile applies: read-only for agent-team-ro.
    let o = ro.ok("observe", json!({"window": w, "full": true})).await;
    let refs = common::refs_by_name(o["text"].as_str().unwrap());
    let e = ro.call("click", json!({"ref": refs["btnA"]})).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "{e}");
    assert_eq!(tb.state()["clicks"]["btnA"], 0);

    // A program loosens the file: the signature breaks → acting calls blocked, reading works.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let f = teamd.join("team-policy.json");
    std::fs::write(&f, std::fs::read_to_string(&f).unwrap().replace("\"deny\"", "\"allow\"")).unwrap();
    let s = core.ok("status", json!({})).await;
    assert_eq!(s["team"]["active"], false, "{s}");
    let e = core.call("run_skill", folder("b")).await;
    assert_eq!(code(&e), "BLOCKED_BY_POLICY", "{e}");
    assert!(e.to_string().contains("could not be verified"), "{e}");
    core.ok("observe", json!({"window": w})).await;
    let r = core.ok("run_skill", json!({"name": "files.list", "args": {"path": work.display().to_string()}})).await;
    assert!(r["text"].is_string());

    // No team policy (an administrator removed it): the user's rules apply again.
    std::fs::remove_file(&f).unwrap();
    assert!(core.ok("status", json!({})).await["team"].is_null());
    core.ok("run_skill", folder("c")).await;
    assert!(work.join("c").is_dir());

    drop(ro);
    core.stop().await;
    let _ = std::fs::remove_dir_all(&base);
}
