//! P7-M3 developer pack slice 1 (ADR 0052), interactive: `dev.check` on the testbed.
mod common;

use common::{Core, Testbed};
use serde_json::json;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: opens the testbed"]
async fn p7m3_dev_check_pass_fail_and_late_text() {
    let mut tb = Testbed::start(260, 180);
    let mut core = Core::start().await;
    let w = tb.window();
    tb.cmd("settext hello world");

    // All pass.
    let r = core
        .ok(
            "run_skill",
            json!({"name": "dev.check", "args": {"window": w, "expect": [
                {"text": "HELLO world"}, {"element": "btnA", "role": "Button"},
                {"title": "Testbed"}, {"no_text": "boom-xyz"}
            ]}}),
        )
        .await;
    assert_eq!(r["pass"], true, "{r}");
    assert!(r["screenshot"].is_null(), "{r}");
    eprintln!("{}", r["text"].as_str().unwrap());

    // A failure: says which check failed and saves a screenshot.
    let r = core
        .ok(
            "run_skill",
            json!({"name": "dev.check", "args": {"window": w, "timeout_ms": 500, "expect": [
                {"text": "hello"}, {"element": "Delete everything", "role": "Button"}
            ]}}),
        )
        .await;
    assert_eq!(r["pass"], false, "{r}");
    assert_eq!(r["checks"][0]["pass"], true, "{r}");
    assert_eq!(r["checks"][1]["pass"], false, "{r}");
    assert!(r["checks"][1]["detail"].as_str().unwrap().contains("btn"), "names what is there: {r}");
    let shot = r["screenshot"].as_str().expect("screenshot path");
    assert!(std::fs::metadata(shot).map(|m| m.len() > 1000).unwrap_or(false), "{shot}");
    let _ = std::fs::remove_file(shot);
    eprintln!("{}", r["text"].as_str().unwrap());

    // A text that appears 1.2 s later: the check waits for it.
    tb.cmd("latertext 1200 arrived-late");
    let r = core
        .ok(
            "run_skill",
            json!({"name": "dev.check", "args": {"window": w, "timeout_ms": 5000, "expect": [{"text": "arrived-late"}]}}),
        )
        .await;
    assert_eq!(r["pass"], true, "{r}");
    assert!(r["ms"].as_u64().unwrap() >= 900, "{r}");

    // Element value and state, and a visible change (the text field updates 0.8 s later).
    tb.cmd("latertext 800 changed-later");
    let r = core
        .ok(
            "run_skill",
            json!({"name": "dev.check", "args": {"window": w, "timeout_ms": 5000, "expect": [
                {"element": "text", "role": "Edit", "value": "changed-later"},
                {"element": "btnA", "enabled": true},
                {"changed": true}
            ]}}),
        )
        .await;
    assert_eq!(r["pass"], true, "{r}");
    eprintln!("{}", r["text"].as_str().unwrap());
    let r = core
        .ok(
            "run_skill",
            json!({"name": "dev.check", "args": {"window": w, "timeout_ms": 600, "screenshot_on_fail": false, "expect": [
                {"element": "text", "value": "something-else"}, {"changed": true}
            ]}}),
        )
        .await;
    assert_eq!(r["pass"], false, "{r}");
    assert!(r["checks"][0]["detail"].as_str().unwrap().contains("changed-later"), "shows the actual value: {r}");
    assert_eq!(r["checks"][1]["pass"], false, "nothing changed: {r}");
    eprintln!("{}", r["text"].as_str().unwrap());

    // Bad arguments are refused before anything is read.
    let e = core
        .call("run_skill", json!({"name": "dev.check", "args": {"window": w, "expect": [{"nope": 1}]}}))
        .await;
    assert_eq!(common::code(&e), "INVALID_ARGS", "{e}");
    core.stop().await;
}
