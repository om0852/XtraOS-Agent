//! P2-M2 interactive tests (docs/08 P2-OCR-*): OCR on a hand-drawn (opaque) window. They open the
//! opaque testbed; P2-OCR-02 clicks with the real mouse. `#[ignore]`: run via
//! tests/phase2/p2m2-gate.ps1 -Interactive, hands off.
mod common;

use std::time::Instant;

use common::{code, tokens, Core, Testbed};
use serde_json::{json, Value};

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

const NAMES: [&str; 4] = ["Start Game", "Options", "Quit", "Score 1234"];

fn text(v: &Value) -> String {
    v["text"].as_str().unwrap_or("").to_string()
}

/// `"Start Game" @cx,cy (x,y wxh)` → (text, cx, cy, x, y, w, h)
fn parse(l: &str) -> Option<(String, i32, i32, i32, i32, i32, i32)> {
    let t = l.strip_prefix('"')?;
    let (name, rest) = t.split_once("\" @")?;
    let (c, rest) = rest.split_once(" (")?;
    let (cx, cy) = c.split_once(',')?;
    let rest = rest.strip_suffix(')')?;
    let (xy, wh) = rest.split_once(' ')?;
    let (x, y) = xy.split_once(',')?;
    let (w, h) = wh.split_once('x')?;
    Some((
        name.to_string(),
        cx.parse().ok()?,
        cy.parse().ok()?,
        x.parse().ok()?,
        y.parse().ok()?,
        w.parse().ok()?,
        h.parse().ok()?,
    ))
}

fn lines(v: &Value) -> Vec<(String, i32, i32, i32, i32, i32, i32)> {
    text(v).lines().filter_map(parse).collect()
}

// P2-OCR-01 + P2-OCR-03 + P2-OCR-04: all drawn texts at the right places; region = window.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn ocr01_03_04_reads_drawn_text_at_the_right_place() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let tb = Testbed::start_with(160, 140, &["--opaque"]);
    let o = core
        .ok("observe", json!({"window": tb.window(), "full": true}))
        .await;
    assert_eq!(o["opaque"], true, "{o}");
    let r = core.ok("ocr", json!({"window": tb.window()})).await;
    let t = text(&r);
    eprintln!("ocr ({} ms, ~{} tokens):\n{t}", r["ms"], tokens(&t));
    let got = lines(&r);
    for name in NAMES {
        let l = got
            .iter()
            .find(|l| l.0 == name)
            .unwrap_or_else(|| panic!("'{name}' not read: {t}"));
        let [rx, ry, rw, rh] = tb.rects[name];
        assert!(
            l.1 >= rx && l.1 <= rx + rw && l.2 >= ry && l.2 <= ry + rh,
            "{name}: centre ({},{}) outside drawn rect {:?}",
            l.1,
            l.2,
            tb.rects[name]
        );
        assert!(
            l.3 >= rx - 8 && l.4 >= ry - 8 && l.3 + l.5 <= rx + rw + 8 && l.4 + l.6 <= ry + rh + 8,
            "{name}: box {:?} not within 8 px of {:?}",
            l,
            tb.rects[name]
        );
    }
    // P2-OCR-03: the same area as a screen region gives the same screen coordinates.
    let wb = core
        .ok("window_bounds", json!({"window": tb.window()}))
        .await;
    let region = json!({"x": wb["x"], "y": wb["y"], "w": wb["w"], "h": wb["h"]});
    let rr = core.ok("ocr", json!({"region": region})).await;
    for name in NAMES {
        let a = got.iter().find(|l| l.0 == name).unwrap();
        let b = lines(&rr)
            .into_iter()
            .find(|l| l.0 == name)
            .unwrap_or_else(|| panic!("region OCR missed {name}: {}", text(&rr)));
        assert!(
            (a.1 - b.1).abs() <= 2 && (a.2 - b.2).abs() <= 2,
            "{name}: {a:?} vs {b:?}"
        );
    }
    // P2-OCR-04: far cheaper than an image.
    let s = core
        .ok("screenshot", json!({"window": tb.window(), "scale": 0.5}))
        .await;
    let img_tokens = s["png_base64"].as_str().unwrap().len() / 4;
    eprintln!(
        "ocr ~{} tokens vs screenshot ~{img_tokens} (base64/4)",
        tokens(&t)
    );
    assert!(
        tokens(&t) * 5 <= img_tokens,
        "OCR text should be <= 1/5 of the image"
    );
    drop(tb);
    core.stop().await;
}

// P2-OCR-02: find a drawn button by OCR, click its centre, only that button counts.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive: clicks with the real mouse"]
async fn ocr02_click_what_ocr_found() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let mut tb = Testbed::start_with(160, 140, &["--opaque"]);
    let r = core
        .ok("ocr", json!({"window": tb.window(), "query": "start game"}))
        .await;
    eprintln!("query: {}", text(&r));
    let hit = lines(&r);
    assert_eq!(hit.len(), 1, "{}", text(&r));
    let (_, cx, cy, ..) = hit[0];
    core.ok("click", json!({"x": cx, "y": cy, "window": tb.window()}))
        .await;
    let st = tb.state();
    assert_eq!(st["clicks"]["Start Game"], 1, "{st}");
    assert_eq!(st["clicks"]["Options"], 0);
    assert_eq!(st["clicks"]["Quit"], 0);
    core.stop().await;
}

// P2-OCR-05 + P2-OCR-06: no match is not an error; bad args; latency.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "interactive"]
async fn ocr05_06_no_match_bad_args_latency() {
    let _s = SERIAL.lock().await;
    let mut core = Core::start().await;
    let tb = Testbed::start_with(160, 140, &["--opaque"]);
    let r = core
        .ok(
            "ocr",
            json!({"window": tb.window(), "query": "zzz nothing"}),
        )
        .await;
    assert_eq!(r["lines"], 0, "{r}");
    assert!(text(&r).contains("-- 0 matching lines"));
    let e = core
        .call("ocr", json!({"window": tb.window(), "max_lines": 0}))
        .await;
    assert_eq!(code(&e), "INVALID_ARGS", "{e}");
    let mut ms = Vec::new();
    for _ in 0..10 {
        let t0 = Instant::now();
        core.ok("ocr", json!({"window": tb.window()})).await;
        ms.push(t0.elapsed().as_millis());
    }
    ms.sort_unstable();
    eprintln!("ocr latency p50 {} ms, max {} ms", ms[5], ms[9]);
    assert!(ms[5] <= 400, "p50 {} ms > 400", ms[5]);
    drop(tb);
    core.stop().await;
}
