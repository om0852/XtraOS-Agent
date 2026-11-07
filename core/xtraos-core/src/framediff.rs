//! `frame_diff` (P3-M1, ADR 0036): which parts of a window (or screen region) changed since
//! the last call, for canvas apps, games and video where UI Automation sees nothing.
//! Background capture (works when the window is covered, not when minimised). Each capture is
//! reduced to a 16 px grid of average colours; changed cells are merged into rectangles in
//! screen coordinates. No pixels are returned (use screenshot / ocr for those).
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::capture::{self, Image};
use crate::input::ActError;

pub const CELL: usize = 16;
/// A cell counts as changed when an average channel moved more than this (0–255).
const THRESHOLD: i32 = 6;
const MAX_REGIONS: usize = 16;
const MAX_FRAMES: usize = 32;

#[derive(Clone, Debug, PartialEq)]
pub struct Sig {
    pub cols: usize,
    pub rows: usize,
    pub img_w: usize,
    pub img_h: usize,
    /// Average B, G, R per cell, row-major.
    pub cells: Vec<[u8; 3]>,
}

struct Frame {
    sig: Sig,
    origin: (i32, i32),
    seq: u64,
}

static FRAMES: Mutex<Option<HashMap<String, Frame>>> = Mutex::new(None);
static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn bad(m: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: m.into(),
        hint: "See frame_diff in docs/04-mcp-tool-contract.md.",
    }
}

/// Grid of average colours.
pub fn signature(img: &Image) -> Sig {
    let cols = img.w.div_ceil(CELL).max(1);
    let rows = img.h.div_ceil(CELL).max(1);
    let mut sum = vec![[0u64; 4]; cols * rows];
    for y in 0..img.h {
        let row = y / CELL;
        let line = &img.bgra[y * img.w * 4..(y + 1) * img.w * 4];
        for (x, px) in line.as_chunks::<4>().0.iter().enumerate() {
            let s = &mut sum[row * cols + x / CELL];
            s[0] += px[0] as u64;
            s[1] += px[1] as u64;
            s[2] += px[2] as u64;
            s[3] += 1;
        }
    }
    let cells = sum
        .iter()
        .map(|s| {
            let n = s[3].max(1);
            [(s[0] / n) as u8, (s[1] / n) as u8, (s[2] / n) as u8]
        })
        .collect();
    Sig {
        cols,
        rows,
        img_w: img.w,
        img_h: img.h,
        cells,
    }
}

/// Changed cells between two signatures (None = different size: everything changed).
pub fn changed_cells(a: &Sig, b: &Sig) -> Option<Vec<bool>> {
    if a.cols != b.cols || a.rows != b.rows || a.img_w != b.img_w || a.img_h != b.img_h {
        return None;
    }
    Some(
        a.cells
            .iter()
            .zip(&b.cells)
            .map(|(p, q)| (0..3).any(|i| (p[i] as i32 - q[i] as i32).abs() > THRESHOLD))
            .collect(),
    )
}

/// Connected changed cells → rectangles in image pixels (x, y, w, h), largest first; beyond
/// MAX_REGIONS the rest are merged into one.
pub fn regions(sig: &Sig, changed: &[bool]) -> Vec<(usize, usize, usize, usize)> {
    let (c, r) = (sig.cols, sig.rows);
    let mut seen = vec![false; changed.len()];
    let mut boxes: Vec<(usize, usize, usize, usize)> = Vec::new(); // cell coords x0,y0,x1,y1
    for start in 0..changed.len() {
        if !changed[start] || seen[start] {
            continue;
        }
        let (mut x0, mut y0, mut x1, mut y1) = (start % c, start / c, start % c, start / c);
        let mut stack = vec![start];
        seen[start] = true;
        while let Some(i) = stack.pop() {
            let (x, y) = (i % c, i / c);
            x0 = x0.min(x);
            x1 = x1.max(x);
            y0 = y0.min(y);
            y1 = y1.max(y);
            // 8-neighbourhood: a diagonal stroke is one region
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= c as i32 || ny >= r as i32 {
                        continue;
                    }
                    let j = ny as usize * c + nx as usize;
                    if changed[j] && !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
        }
        boxes.push((x0, y0, x1, y1));
    }
    boxes.sort_by_key(|b| std::cmp::Reverse((b.2 - b.0 + 1) * (b.3 - b.1 + 1)));
    if boxes.len() > MAX_REGIONS {
        let rest = boxes.split_off(MAX_REGIONS - 1);
        let m = rest.iter().fold(rest[0], |a, b| {
            (a.0.min(b.0), a.1.min(b.1), a.2.max(b.2), a.3.max(b.3))
        });
        boxes.push(m);
    }
    boxes
        .into_iter()
        .map(|(x0, y0, x1, y1)| {
            let px = x0 * CELL;
            let py = y0 * CELL;
            let pw = ((x1 + 1) * CELL).min(sig.img_w) - px;
            let ph = ((y1 + 1) * CELL).min(sig.img_h) - py;
            (px, py, pw, ph)
        })
        .collect()
}

fn key(p: &Value) -> String {
    if let Some(r) = p.get("region").filter(|v| !v.is_null()) {
        return format!("region:{}", r);
    }
    match p.get("window") {
        Some(v) if !v.is_null() => crate::winmgr::resolve_window(v)
            .map(crate::winmgr::window_ref)
            .unwrap_or_else(|_| v.to_string()),
        _ => crate::winmgr::window_ref(crate::winmgr::foreground_root()),
    }
}

fn store(k: String, sig: Sig, origin: (i32, i32)) {
    let mut g = FRAMES.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(HashMap::new);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    m.insert(k, Frame { sig, origin, seq });
    while m.len() > MAX_FRAMES {
        let oldest = m.iter().min_by_key(|(_, f)| f.seq).map(|(k, _)| k.clone());
        match oldest {
            Some(o) => {
                m.remove(&o);
            }
            None => break,
        }
    }
}

/// The `frame_diff` method.
pub fn frame_diff(p: &Value) -> Result<Value, ActError> {
    let wait_ms = match p.get("wait_ms") {
        None | Some(Value::Null) => 0,
        Some(v) => v
            .as_u64()
            .filter(|n| *n <= 60_000)
            .ok_or_else(|| bad("'wait_ms' must be 0..=60000"))?,
    };
    let min_pct = match p.get("min_pct") {
        None | Some(Value::Null) => 0.5,
        Some(v) => v
            .as_f64()
            .filter(|n| (0.0..=100.0).contains(n))
            .ok_or_else(|| bad("'min_pct' must be 0..=100"))?,
    };
    let reset = p.get("reset").and_then(Value::as_bool).unwrap_or(false);
    let t0 = Instant::now();
    let k = key(p);
    let (img, origin, window) = capture::grab(p)?;
    let sig = signature(&img);
    let prev = if reset {
        None
    } else {
        FRAMES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(|m| m.get(&k).map(|f| (f.sig.clone(), f.origin)))
    };
    let Some((base, base_origin)) = prev else {
        store(k, sig, origin);
        return Ok(json!({
            "text": "baseline set: call frame_diff again to see what changed",
            "baseline": true, "changed": false, "changed_pct": 0.0, "regions": [], "window": window,
        }));
    };
    let mut cur = (sig, origin);
    let deadline = t0 + Duration::from_millis(wait_ms);
    let (cells, pct) = loop {
        let cells = if cur.1 != base_origin {
            None // the window moved: report as fully changed
        } else {
            changed_cells(&base, &cur.0)
        };
        let pct = match &cells {
            None => 100.0,
            Some(c) => 100.0 * c.iter().filter(|x| **x).count() as f64 / c.len().max(1) as f64,
        };
        if pct >= min_pct || Instant::now() >= deadline {
            break (cells, pct);
        }
        crate::safety::check()?;
        std::thread::sleep(Duration::from_millis(50));
        let (img, o, _) = capture::grab(p)?;
        cur = (signature(&img), o);
    };
    let (sig, origin) = cur;
    let changed = pct >= min_pct;
    let regs: Vec<(usize, usize, usize, usize)> = match &cells {
        None => vec![(0, 0, sig.img_w, sig.img_h)],
        Some(c) if changed => regions(&sig, c),
        Some(_) => Vec::new(),
    };
    store(k, sig, origin);
    let screen: Vec<Value> = regs
        .iter()
        .map(|&(x, y, w, h)| {
            json!({"x": origin.0 + x as i32, "y": origin.1 + y as i32, "w": w, "h": h})
        })
        .collect();
    let text = if changed {
        format!(
            "changed {pct:.1}% in {} region(s): {}",
            screen.len(),
            screen
                .iter()
                .map(|r| format!("({},{} {}x{})", r["x"], r["y"], r["w"], r["h"]))
                .collect::<Vec<_>>()
                .join(" ")
        )
    } else if wait_ms > 0 {
        format!("no change above {min_pct}% within {wait_ms} ms")
    } else {
        "no change".to_string()
    };
    Ok(json!({
        "text": text,
        "baseline": false,
        "changed": changed,
        "changed_pct": (pct * 10.0).round() / 10.0,
        "regions": screen,
        "window": window,
        "ms": t0.elapsed().as_millis() as u64,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize, paint: &[(usize, usize, usize, usize, u8)]) -> Image {
        let mut bgra = vec![200u8; w * h * 4];
        for &(x0, y0, pw, ph, v) in paint {
            for y in y0..y0 + ph {
                for x in x0..x0 + pw {
                    let i = (y * w + x) * 4;
                    bgra[i..i + 3].copy_from_slice(&[v, v, v]);
                }
            }
        }
        Image { w, h, bgra }
    }

    #[test]
    fn identical_frames_have_no_change() {
        let a = signature(&img(100, 70, &[]));
        let c = changed_cells(&a, &a.clone()).unwrap();
        assert!(c.iter().all(|x| !x));
        assert_eq!((a.cols, a.rows), (7, 5), "partial cells at the edges count");
    }

    #[test]
    fn one_block_and_two_blocks() {
        let a = signature(&img(160, 160, &[]));
        let b = signature(&img(160, 160, &[(40, 40, 20, 20, 0)]));
        let c = changed_cells(&a, &b).unwrap();
        let r = regions(&b, &c);
        assert_eq!(
            r,
            vec![(32, 32, 32, 32)],
            "20 px block at 40,40 spans cells 2..3"
        );
        let d = signature(&img(160, 160, &[(0, 0, 10, 10, 0), (130, 130, 20, 20, 0)]));
        let r = regions(&d, &changed_cells(&a, &d).unwrap());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0], (128, 128, 32, 32), "larger first");
        // A small sprite (4x4) still flips its cell.
        let s = signature(&img(160, 160, &[(70, 70, 4, 4, 0)]));
        assert_eq!(regions(&s, &changed_cells(&a, &s).unwrap()).len(), 1);
    }

    #[test]
    fn diagonal_stroke_is_one_region_and_cap_merges() {
        let a = signature(&img(320, 320, &[]));
        let stroke: Vec<_> = (0..10).map(|i| (i * 16, i * 16, 8, 8, 0u8)).collect();
        let b = signature(&img(320, 320, &stroke));
        assert_eq!(regions(&b, &changed_cells(&a, &b).unwrap()).len(), 1);
        let dots: Vec<_> = (0..40)
            .map(|i| ((i % 10) * 32, (i / 10) * 64, 4, 4, 0u8))
            .collect();
        let c = signature(&img(320, 320, &dots));
        assert_eq!(
            regions(&c, &changed_cells(&a, &c).unwrap()).len(),
            MAX_REGIONS
        );
        assert!(
            changed_cells(&a, &signature(&img(300, 320, &[]))).is_none(),
            "resize"
        );
    }
}
