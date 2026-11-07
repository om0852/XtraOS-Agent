//! Phase 4 part 1 (ADR 0042): audit export on a private log. No windows, no input.
mod common;

use common::{code, Core};
use serde_json::{json, Value};
use xtraos_core::actionlog::{ActionLog, Entry};

#[tokio::test(flavor = "multi_thread")]
async fn ax01_audit_export_csv_json_chain_and_no_overwrite() {
    let home = std::env::temp_dir().join(format!("xtraos-ax-home-{}", common::rand_hex(3)));
    let outdir = std::env::temp_dir().join(format!("xtraos-ax-out-{}", common::rand_hex(3)));
    std::fs::create_dir_all(&outdir).unwrap();
    std::env::set_var("XTRAOS_HOME", &home);
    let log = ActionLog::create(&home.join("logs")).unwrap();
    let now = xtraos_core::protocol::now_ms();
    for (m, ok, code_) in [
        ("click", true, None),
        ("type", false, Some("BLOCKED_BY_POLICY")),
        ("observe", true, None),
    ] {
        log.record(&Entry {
            ts_ms: now,
            session: Some("s1"),
            client: Some("t,est"),
            method: m,
            ok,
            code: code_,
            ms: 1.5,
            policy: Some("allow"),
        });
    }
    let last_hash = {
        let t = std::fs::read_to_string(log.path()).unwrap();
        let v: Value = serde_json::from_str(t.lines().last().unwrap()).unwrap();
        v["hash"].as_str().unwrap().to_string()
    };
    drop(log);
    let mut core = Core::start().await;
    let csv = outdir.join("audit.csv");
    let r = core
        .ok("audit_export", json!({"out": csv.to_string_lossy()}))
        .await;
    eprintln!("AX-01: {}", r["text"]);
    assert_eq!(r["entries"], 3);
    assert_eq!(r["chain_ok"], true);
    assert_eq!(r["head_hash"], last_hash.as_str());
    let body = std::fs::read_to_string(&csv).unwrap();
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(
        lines[0],
        "seq,time_utc,session,client,method,ok,code,ms,policy,hash,prev"
    );
    assert_eq!(lines.len(), 4);
    assert!(
        lines[2].contains("BLOCKED_BY_POLICY") && lines[2].contains("\"t,est\""),
        "{}",
        lines[2]
    );
    let again = core
        .call("audit_export", json!({"out": csv.to_string_lossy()}))
        .await;
    assert_eq!(code(&again), "INVALID_ARGS", "never overwrites: {again}");
    let js = outdir.join("audit.json");
    let r = core
        .ok(
            "audit_export",
            json!({"format": "json", "out": js.to_string_lossy(), "from": "2000-01-01"}),
        )
        .await;
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&js).unwrap()).unwrap();
    assert_eq!(v["entries"].as_array().unwrap().len(), 3);
    assert_eq!(v["chain"]["ok"], true);
    assert_eq!(r["entries"], 3);
    let none = core
        .ok(
            "audit_export",
            json!({"out": outdir.join("none.csv").to_string_lossy(), "to": "2000-01-01"}),
        )
        .await;
    assert_eq!(none["entries"], 0, "date filter");
    let protected = core
        .call(
            "audit_export",
            json!({"out": home.join("x.csv").to_string_lossy()}),
        )
        .await;
    assert_eq!(
        code(&protected),
        "BLOCKED_BY_POLICY",
        "the XtraOS data folder is protected: {protected}"
    );
    core.stop().await;
    std::env::remove_var("XTRAOS_HOME");
    let _ = std::fs::remove_dir_all(home);
    let _ = std::fs::remove_dir_all(outdir);
}
