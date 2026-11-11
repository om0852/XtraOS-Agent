//! Local vision (ADR 0070): pictures described as compact text, computed on the PC, so an
//! agent does not have to send a screenshot to its model. No network, no paid service.

use std::path::Path;

use serde_json::{json, Value};

use crate::capture::Image;
use crate::input::ActError;

fn bad(m: impl Into<String>) -> ActError {
    ActError { code: "INVALID_ARGS", message: m.into(), hint: "See list_skills for vision.* arguments." }
}

// ---------------------------------------------------------------------------------------------
// Named colours (pure)

/// (name, legend letter, r, g, b). Letters are unique.
pub const PALETTE: &[(&str, char, u8, u8, u8)] = &[
    ("white", 'W', 250, 250, 250),
    ("light grey", 'l', 200, 200, 200),
    ("grey", 'g', 128, 128, 128),
    ("dark grey", 'd', 64, 64, 64),
    ("black", 'K', 12, 12, 12),
    ("red", 'R', 220, 30, 35),
    ("dark red", 'r', 130, 20, 25),
    ("orange", 'O', 245, 130, 40),
    ("yellow", 'Y', 250, 230, 30),
    ("gold", 'o', 225, 180, 40),
    ("cream", 'c', 240, 230, 180),
    ("brown", 'N', 150, 95, 55),
    ("green", 'G', 35, 175, 75),
    ("dark green", 'n', 20, 90, 40),
    ("lime", 'L', 170, 225, 60),
    ("teal", 't', 20, 150, 150),
    ("cyan", 'C', 70, 200, 230),
    ("blue", 'B', 40, 110, 230),
    ("navy", 'v', 25, 35, 100),
    ("indigo", 'I', 70, 70, 200),
    ("purple", 'P', 150, 60, 180),
    ("pink", 'p', 245, 150, 190),
];

/// Index of the nearest named colour (weighted RGB distance; "redmean" approximation).
pub fn nearest(r: u8, g: u8, b: u8) -> usize {
    let mut best = (0, f64::MAX);
    for (i, &(_, _, pr, pg, pb)) in PALETTE.iter().enumerate() {
        let rm = (r as f64 + pr as f64) / 2.0;
        let (dr, dg, db) = (r as f64 - pr as f64, g as f64 - pg as f64, b as f64 - pb as f64);
        let d = (2.0 + rm / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - rm) / 256.0) * db * db;
        if d < best.1 {
            best = (i, d);
        }
    }
    best.0
}

fn palette_index(name: &str) -> Option<usize> {
    PALETTE.iter().position(|p| p.0.eq_ignore_ascii_case(name.trim()))
}

/// A reduced copy as palette indices (≤ `max_w` wide; each cell the colour of its centre pixel).
pub struct Labels {
    pub w: usize,
    pub h: usize,
    /// screen pixels per label cell
    pub k: f64,
    pub idx: Vec<u8>,
}

pub fn labels(img: &Image, max_w: usize) -> Labels {
    let k = (img.w as f64 / max_w as f64).max(1.0);
    let (w, h) = (((img.w as f64) / k).floor().max(1.0) as usize, ((img.h as f64) / k).floor().max(1.0) as usize);
    let mut idx = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let sx = ((x as f64 + 0.5) * k) as usize;
            let sy = ((y as f64 + 0.5) * k) as usize;
            let o = (sy.min(img.h - 1) * img.w + sx.min(img.w - 1)) * 4;
            idx.push(nearest(img.bgra[o + 2], img.bgra[o + 1], img.bgra[o]) as u8);
        }
    }
    Labels { w, h, k, idx }
}

// ---------------------------------------------------------------------------------------------
// vision.colors (pure core)

/// Shares of each named colour, largest first, at least `min_pct`.
pub fn shares(l: &Labels, min_pct: f64) -> Vec<(usize, f64)> {
    let mut c = vec![0usize; PALETTE.len()];
    for &i in &l.idx {
        c[i as usize] += 1;
    }
    let n = l.idx.len().max(1) as f64;
    let mut v: Vec<(usize, f64)> = c.iter().enumerate().map(|(i, &k)| (i, k as f64 * 100.0 / n)).filter(|(_, p)| *p >= min_pct).collect();
    v.sort_by(|a, b| b.1.total_cmp(&a.1));
    v
}

/// The grid map: `cols × rows` letters, each the main colour of its cell.
pub fn grid(l: &Labels, cols: usize, rows: usize) -> Vec<String> {
    let mut out = Vec::new();
    for gy in 0..rows {
        let mut line = String::new();
        for gx in 0..cols {
            let (x0, x1) = (gx * l.w / cols, ((gx + 1) * l.w / cols).max(gx * l.w / cols + 1));
            let (y0, y1) = (gy * l.h / rows, ((gy + 1) * l.h / rows).max(gy * l.h / rows + 1));
            let mut c = vec![0usize; PALETTE.len()];
            for y in y0..y1.min(l.h) {
                for x in x0..x1.min(l.w) {
                    c[l.idx[y * l.w + x] as usize] += 1;
                }
            }
            let best = c.iter().enumerate().max_by_key(|(_, &k)| k).map(|(i, _)| i).unwrap_or(0);
            line.push(PALETTE[best].1);
        }
        out.push(line);
    }
    out
}

fn parse_grid(a: &Value) -> Result<(usize, usize), ActError> {
    let s = a.get("grid").and_then(Value::as_str).unwrap_or("8x6");
    let (c, r) = s.split_once(['x', 'X']).ok_or_else(|| bad("'grid' is like \"8x6\""))?;
    match (c.trim().parse::<usize>(), r.trim().parse::<usize>()) {
        (Ok(c), Ok(r)) if (1..=24).contains(&c) && (1..=16).contains(&r) => Ok((c, r)),
        _ => Err(bad("'grid' columns 1..24, rows 1..16 (e.g. \"8x6\")")),
    }
}

pub fn colors_text(l: &Labels, cols: usize, rows: usize, min_pct: f64) -> (String, Value) {
    let sh = shares(l, min_pct);
    let main: Vec<String> = sh.iter().map(|(i, p)| format!("{} {:.0}%", PALETTE[*i].0, p)).collect();
    let g = grid(l, cols, rows);
    let mut used: Vec<char> = g.iter().flat_map(|r| r.chars()).collect();
    used.sort_unstable();
    used.dedup();
    let legend: Vec<String> = used.iter().filter_map(|c| PALETTE.iter().find(|p| p.1 == *c).map(|p| format!("{c}={}", p.0))).collect();
    let text = format!("colours: {}\nmap {cols}x{rows} (top row first; {}):\n{}", main.join(", "), legend.join(" "), g.join("\n"));
    let data = json!({
        "colors": sh.iter().map(|(i, p)| json!({"name": PALETTE[*i].0, "pct": (p * 10.0).round() / 10.0})).collect::<Vec<_>>(),
        "grid": g,
    });
    (text, data)
}

// ---------------------------------------------------------------------------------------------
// vision.regions (pure core)

#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    pub color: usize,
    /// label-grid box: x0, y0, x1 (excl), y1 (excl)
    pub bx: (usize, usize, usize, usize),
    pub cells: usize,
}

impl Region {
    /// round / box / thin / blob, from the fill of the bounding box and the aspect ratio.
    pub fn shape(&self) -> &'static str {
        let (w, h) = ((self.bx.2 - self.bx.0) as f64, (self.bx.3 - self.bx.1) as f64);
        let fill = self.cells as f64 / (w * h).max(1.0);
        let aspect = w.max(h) / w.min(h).max(1.0);
        if aspect >= 3.0 && fill > 0.25 {
            "thin"
        } else if fill >= 0.9 {
            "box"
        } else if (0.68..0.9).contains(&fill) && aspect < 1.4 {
            "round"
        } else {
            "blob"
        }
    }
}

/// 4-connected regions of the wanted colours, largest first.
pub fn find_regions(l: &Labels, wanted: &[bool], min_cells: usize) -> Vec<Region> {
    let mut seen = vec![false; l.idx.len()];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    for start in 0..l.idx.len() {
        let c = l.idx[start] as usize;
        if seen[start] || !wanted[c] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (usize::MAX, usize::MAX, 0, 0, 0);
        while let Some(p) = stack.pop() {
            let (x, y) = (p % l.w, p / l.w);
            n += 1;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
            let mut push = |q: usize| {
                if !seen[q] && l.idx[q] as usize == c {
                    seen[q] = true;
                    stack.push(q);
                }
            };
            if x > 0 {
                push(p - 1);
            }
            if x + 1 < l.w {
                push(p + 1);
            }
            if y > 0 {
                push(p - l.w);
            }
            if y + 1 < l.h {
                push(p + l.w);
            }
        }
        if n >= min_cells {
            out.push(Region { color: c, bx: (x0, y0, x1, y1), cells: n });
        }
    }
    out.sort_by(|a, b| b.cells.cmp(&a.cells));
    out
}

pub fn regions_text(l: &Labels, regs: &[Region], origin: (i32, i32), max: usize, background: Option<usize>) -> (String, Value) {
    let total = (l.w * l.h).max(1) as f64;
    let to_screen = |gx: usize, gy: usize| (origin.0 + (gx as f64 * l.k) as i32, origin.1 + (gy as f64 * l.k) as i32);
    let mut lines = Vec::new();
    let mut data = Vec::new();
    for r in regs.iter().take(max) {
        let (sx0, sy0) = to_screen(r.bx.0, r.bx.1);
        let (sx1, sy1) = to_screen(r.bx.2, r.bx.3);
        let (w, h) = (sx1 - sx0, sy1 - sy0);
        let pct = r.cells as f64 * 100.0 / total;
        lines.push(format!(
            "{} {} @{},{} {}x{} centre {},{} {:.1}%",
            PALETTE[r.color].0,
            r.shape(),
            sx0,
            sy0,
            w,
            h,
            sx0 + w / 2,
            sy0 + h / 2,
            pct
        ));
        data.push(json!({"color": PALETTE[r.color].0, "shape": r.shape(), "x": sx0, "y": sy0, "w": w, "h": h, "pct": (pct * 10.0).round() / 10.0}));
    }
    // Per colour: how many pieces, so "18 yellow petals" is visible even past the cap.
    let mut per: Vec<(usize, usize)> = Vec::new();
    for r in regs {
        match per.iter_mut().find(|p| p.0 == r.color) {
            Some(p) => p.1 += 1,
            None => per.push((r.color, 1)),
        }
    }
    let counts: Vec<String> = per.iter().map(|(c, n)| format!("{n} {}", PALETTE[*c].0)).collect();
    let mut text = format!("{} region(s): {}", regs.len(), counts.join(", "));
    if let Some(b) = background {
        text.push_str(&format!("; background {}", PALETTE[b].0));
    }
    text.push('\n');
    text.push_str(&lines.join("\n"));
    if regs.len() > max {
        text.push_str(&format!("\n… {} smaller region(s) not listed", regs.len() - max));
    }
    (text, json!({"regions": data, "total": regs.len()}))
}

// ---------------------------------------------------------------------------------------------
// vision.find_image (pure core)

pub struct Gray {
    pub w: usize,
    pub h: usize,
    pub px: Vec<f32>,
}

/// Grayscale, reduced by block averaging with factor `k` (1 = full size).
pub fn gray_bgra(img: &Image, k: usize) -> Gray {
    let (w, h) = ((img.w / k).max(1), (img.h / k).max(1));
    let mut px = vec![0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut s = 0f32;
            for dy in 0..k {
                for dx in 0..k {
                    let (sx, sy) = ((x * k + dx).min(img.w - 1), (y * k + dy).min(img.h - 1));
                    let o = (sy * img.w + sx) * 4;
                    s += 0.299 * img.bgra[o + 2] as f32 + 0.587 * img.bgra[o + 1] as f32 + 0.114 * img.bgra[o] as f32;
                }
            }
            px[y * w + x] = s / (k * k) as f32;
        }
    }
    Gray { w, h, px }
}

/// Normalised cross-correlation of template `t` at (x, y) in `g` (−1..1; 1 = identical).
pub fn ncc(g: &Gray, t: &Gray, x: usize, y: usize) -> f32 {
    let n = (t.w * t.h) as f32;
    let (mut sg, mut st) = (0f32, 0f32);
    for ty in 0..t.h {
        for tx in 0..t.w {
            sg += g.px[(y + ty) * g.w + x + tx];
            st += t.px[ty * t.w + tx];
        }
    }
    let (mg, mt) = (sg / n, st / n);
    let (mut num, mut dg, mut dt) = (0f32, 0f32, 0f32);
    for ty in 0..t.h {
        for tx in 0..t.w {
            let a = g.px[(y + ty) * g.w + x + tx] - mg;
            let b = t.px[ty * t.w + tx] - mt;
            num += a * b;
            dg += a * a;
            dt += b * b;
        }
    }
    if dg < 1e-3 || dt < 1e-3 {
        // flat areas: identical only when both are flat and equal in brightness
        return if dg < 1e-3 && dt < 1e-3 && (mg - mt).abs() < 4.0 { 1.0 } else { 0.0 };
    }
    num / (dg.sqrt() * dt.sqrt())
}

/// Matches (x, y, score) at full scale, best first, no two closer than half the template.
pub fn match_template(img: &Image, tpl: &Image, threshold: f32, max: usize) -> Vec<(usize, usize, f32)> {
    if tpl.w > img.w || tpl.h > img.h {
        return Vec::new();
    }
    // Coarse pass on a reduced copy, then refine at full size. The largest reduction that keeps
    // the template ≥ 6 px wide there (a full window then costs well under a second).
    let k = [8usize, 4, 2, 1].into_iter().find(|k| tpl.w / k >= 6 && tpl.h / k >= 6).unwrap_or(1);
    let (g, t) = (gray_bgra(img, k), gray_bgra(tpl, k));
    let mut cand: Vec<(usize, usize, f32)> = Vec::new();
    if t.w <= g.w && t.h <= g.h {
        for y in 0..=(g.h - t.h) {
            for x in 0..=(g.w - t.w) {
                cand.push((x, y, ncc(&g, &t, x, y)));
            }
        }
    }
    // The reduced copy blurs fine detail, so the coarse score is only a ranking: keep the best
    // positions whatever their score and judge them at full size.
    cand.sort_by(|a, b| b.2.total_cmp(&a.2));
    cand.truncate(60);
    let (gf, tf) = (gray_bgra(img, 1), gray_bgra(tpl, 1));
    let mut found: Vec<(usize, usize, f32)> = Vec::new();
    for (cx, cy, _) in cand {
        let (fx, fy) = (cx * k, cy * k);
        if found.iter().any(|f| (f.0 as i64 - fx as i64).unsigned_abs() < (tpl.w / 2) as u64 && (f.1 as i64 - fy as i64).unsigned_abs() < (tpl.h / 2) as u64) {
            continue;
        }
        let mut best = (fx, fy, -1f32);
        let r = k as i64;
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (fx as i64 + dx, fy as i64 + dy);
                if x < 0 || y < 0 || x as usize + tf.w > gf.w || y as usize + tf.h > gf.h {
                    continue;
                }
                let s = ncc(&gf, &tf, x as usize, y as usize);
                if s > best.2 {
                    best = (x as usize, y as usize, s);
                }
            }
        }
        if best.2 >= threshold {
            found.push(best);
            if found.len() >= max {
                break;
            }
        }
    }
    found.sort_by(|a, b| b.2.total_cmp(&a.2));
    found
}

fn load_png(path: &Path) -> Result<Image, ActError> {
    let f = std::fs::File::open(path).map_err(|_| bad(format!("{} does not exist", path.display())))?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(f));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut rd = dec.read_info().map_err(|e| bad(format!("{} is not a PNG: {e}", path.display())))?;
    let mut buf = vec![0u8; rd.output_buffer_size().ok_or_else(|| bad("PNG too large"))?];
    let info = rd.next_frame(&mut buf).map_err(|e| bad(format!("{}: {e}", path.display())))?;
    let (w, h) = (info.width as usize, info.height as usize);
    if w == 0 || h == 0 || w > 512 || h > 512 {
        return Err(bad("the image to find must be 1..512 px on each side"));
    }
    let ch = match info.color_type {
        png::ColorType::Rgba => 4,
        png::ColorType::Rgb => 3,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Grayscale => 1,
        png::ColorType::Indexed => return Err(bad("indexed PNG not supported")),
    };
    let mut bgra = vec![255u8; w * h * 4];
    for i in 0..w * h {
        let s = &buf[i * ch..i * ch + ch];
        let (r, g, b) = match ch {
            1 | 2 => (s[0], s[0], s[0]),
            _ => (s[0], s[1], s[2]),
        };
        bgra[i * 4] = b;
        bgra[i * 4 + 1] = g;
        bgra[i * 4 + 2] = r;
    }
    Ok(Image { w, h, bgra })
}

// ---------------------------------------------------------------------------------------------
// The skills

/// The picture to analyse: `window` (captured even when covered), `region` (what is on screen
/// there), or both: that area of the window's own capture (works while the window is covered).
fn capture(a: &Value) -> Result<crate::capture::Grab, ActError> {
    let win = a.get("window").filter(|v| !v.is_null());
    let reg = a.get("region").filter(|v| !v.is_null());
    match (win, reg) {
        (Some(w), Some(r)) => {
            let (img, origin, wref) = crate::capture::grab(&json!({"window": w}))?;
            let g = |k: &str| r.get(k).and_then(Value::as_i64).ok_or_else(|| bad("'region' is {x, y, w, h} (integers)"));
            let (x, y, w, h) = (g("x")?, g("y")?, g("w")?, g("h")?);
            let (cx, cy) = ((x - origin.0 as i64).max(0) as usize, (y - origin.1 as i64).max(0) as usize);
            if w <= 0 || h <= 0 || cx >= img.w || cy >= img.h {
                return Err(bad("'region' is outside the window"));
            }
            let (cw, ch) = ((w as usize).min(img.w - cx), (h as usize).min(img.h - cy));
            let part = crate::capture::crop(&img, cx, cy, cw, ch);
            Ok((part, (origin.0 + cx as i32, origin.1 + cy as i32), wref))
        }
        _ => {
            let mut p = json!({});
            if let Some(w) = win {
                p["window"] = w.clone();
            }
            if let Some(r) = reg {
                p["region"] = r.clone();
            }
            crate::capture::grab(&p)
        }
    }
}

/// Brightness spread of a picture (pure): a flat patch has none and matches anywhere.
pub fn detail(img: &Image) -> f32 {
    let g = gray_bgra(img, 1);
    let n = g.px.len().max(1) as f32;
    let m = g.px.iter().sum::<f32>() / n;
    (g.px.iter().map(|v| (v - m) * (v - m)).sum::<f32>() / n).sqrt()
}

pub fn run(op: &str, a: &Value) -> Result<Value, ActError> {
    match op {
        "vision.colors" => {
            let (cols, rows) = parse_grid(a)?;
            let min_pct = a.get("min_pct").and_then(Value::as_f64).unwrap_or(1.0).clamp(0.1, 50.0);
            let (img, origin, win) = capture(a)?;
            let l = labels(&img, 320);
            let (text, mut data) = colors_text(&l, cols, rows, min_pct);
            data["text"] = json!(format!("{} {}x{} at {},{}\n{text}", win.as_deref().unwrap_or("region"), img.w, img.h, origin.0, origin.1));
            Ok(data)
        }
        "vision.regions" => {
            let (img, origin, win) = capture(a)?;
            let l = labels(&img, 480);
            let max = a.get("max").and_then(Value::as_u64).unwrap_or(40).clamp(1, 200) as usize;
            let min_pct = a.get("min_area").and_then(Value::as_f64).unwrap_or(0.02).clamp(0.0, 50.0);
            let min_cells = ((l.w * l.h) as f64 * min_pct / 100.0).ceil().max(2.0) as usize;
            let mut wanted = vec![false; PALETTE.len()];
            let background;
            match a.get("colors").and_then(Value::as_array) {
                Some(names) if !names.is_empty() => {
                    for n in names {
                        let s = n.as_str().unwrap_or("");
                        let i = palette_index(s).ok_or_else(|| bad(format!("unknown colour '{s}' (names: {})", PALETTE.iter().map(|p| p.0).collect::<Vec<_>>().join(", "))))?;
                        wanted[i] = true;
                    }
                    background = None;
                }
                _ => {
                    let sh = shares(&l, 0.0);
                    let bg = sh.first().map(|s| s.0);
                    for (i, w) in wanted.iter_mut().enumerate() {
                        *w = Some(i) != bg;
                    }
                    background = bg;
                }
            }
            let regs = find_regions(&l, &wanted, min_cells);
            let (text, mut data) = regions_text(&l, &regs, origin, max, background);
            data["text"] = json!(format!("{} {}x{} at {},{} (approximate: colours named to the nearest of 22)\n{text}", win.as_deref().unwrap_or("region"), img.w, img.h, origin.0, origin.1));
            Ok(data)
        }
        "vision.find_image" => {
            let raw = a.get("image").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| bad("'image' (a PNG path) is required"))?;
            let tpl = load_png(&crate::skills::checked(raw)?)?;
            // A flat patch (one colour) "matches" everywhere: refuse it instead of guessing.
            if detail(&tpl) < 6.0 {
                return Err(bad("the image to find has no detail (one flat colour); snip an area with edges or text, e.g. an icon"));
            }
            let threshold = a.get("threshold").and_then(Value::as_f64).unwrap_or(0.9).clamp(0.5, 1.0) as f32;
            let max = a.get("max").and_then(Value::as_u64).unwrap_or(5).clamp(1, 50) as usize;
            let (img, origin, win) = capture(a)?;
            let t0 = std::time::Instant::now();
            let found = match_template(&img, &tpl, threshold, max);
            let lines: Vec<String> = found
                .iter()
                .map(|(x, y, s)| format!("@{},{} {}x{} centre {},{} score {:.2}", origin.0 + *x as i32, origin.1 + *y as i32, tpl.w, tpl.h, origin.0 + *x as i32 + tpl.w as i32 / 2, origin.1 + *y as i32 + tpl.h as i32 / 2, s))
                .collect();
            Ok(json!({
                "text": format!("{} match(es) of {}x{} in {} ({} ms){}{}", found.len(), tpl.w, tpl.h, win.as_deref().unwrap_or("region"), t0.elapsed().as_millis(), if lines.is_empty() { "" } else { "\n" }, lines.join("\n")),
                "matches": found.iter().map(|(x, y, s)| json!({"x": origin.0 + *x as i32, "y": origin.1 + *y as i32, "w": tpl.w, "h": tpl.h, "score": (s * 100.0).round() / 100.0})).collect::<Vec<_>>(),
            }))
        }
        "vision.snip" => {
            let raw = a.get("path").and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| bad("'path' (.png) is required"))?;
            let path = crate::skills::checked(raw)?;
            if path.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some("png".into()) {
                return Err(bad("'path' must end in .png"));
            }
            if path.exists() {
                return Err(bad(format!("{} already exists (never overwrites)", path.display())));
            }
            if a.get("region").is_none_or(Value::is_null) {
                return Err(bad("'region' {x, y, w, h} is required"));
            }
            let (img, origin, _) = capture(a)?;
            if img.w > 512 || img.h > 512 {
                return Err(bad("a snip is at most 512 x 512 (it is meant as an image to find)"));
            }
            let (w, h, rgb) = crate::capture::downscale_rgb(&img, 1.0);
            let png = crate::capture::png_rgb(w, h, &rgb)?;
            std::fs::write(&path, png).map_err(|e| bad(format!("could not write {}: {e}", path.display())))?;
            Ok(json!({"text": format!("ok saved {w}x{h} from {},{} to {}", origin.0, origin.1, path.display()), "path": path.display().to_string()}))
        }
        o => Err(bad(format!("unknown vision op '{o}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(w: usize, h: usize, f: impl Fn(usize, usize) -> (u8, u8, u8)) -> Image {
        let mut bgra = vec![255u8; w * h * 4];
        for y in 0..h {
            for x in 0..w {
                let (r, g, b) = f(x, y);
                let o = (y * w + x) * 4;
                bgra[o] = b;
                bgra[o + 1] = g;
                bgra[o + 2] = r;
            }
        }
        Image { w, h, bgra }
    }

    fn name(i: usize) -> &'static str {
        PALETTE[i].0
    }

    #[test]
    fn colours_are_named() {
        assert_eq!(name(nearest(255, 255, 255)), "white");
        assert_eq!(name(nearest(0, 0, 0)), "black");
        assert_eq!(name(nearest(255, 242, 0)), "yellow");
        assert_eq!(name(nearest(255, 127, 39)), "orange");
        assert_eq!(name(nearest(185, 122, 87)), "brown");
        assert_eq!(name(nearest(34, 177, 76)), "green");
        assert_eq!(name(nearest(63, 72, 204)), "indigo");
        let mut letters: Vec<char> = PALETTE.iter().map(|p| p.1).collect();
        letters.sort_unstable();
        letters.dedup();
        assert_eq!(letters.len(), PALETTE.len(), "legend letters are unique");
    }

    /// A white picture with a brown disc in the middle, a green bar below, an orange box right.
    fn scene() -> Image {
        img(200, 100, |x, y| {
            let (dx, dy) = (x as f64 - 60.0, y as f64 - 40.0);
            if dx * dx + dy * dy <= 400.0 {
                (150, 95, 55)
            } else if (10..110).contains(&x) && (80..86).contains(&y) {
                (35, 175, 75)
            } else if (150..180).contains(&x) && (20..50).contains(&y) {
                (245, 130, 40)
            } else {
                (255, 255, 255)
            }
        })
    }

    #[test]
    fn colour_shares_and_grid_map() {
        let l = labels(&scene(), 320);
        assert_eq!((l.w, l.h), (200, 100));
        let sh = shares(&l, 1.0);
        assert_eq!(name(sh[0].0), "white");
        assert!(sh.iter().any(|s| name(s.0) == "brown"));
        let g = grid(&l, 4, 2);
        assert_eq!(g, vec!["WWWW".to_string(), "WWWW".to_string()], "the shapes are small next to the white");
        let (text, _) = colors_text(&l, 20, 10, 1.0);
        assert!(text.contains("N=brown") && text.contains("G=green") && text.contains("W=white"), "{text}");
        assert!(text.starts_with("colours: white"), "{text}");
    }

    #[test]
    fn regions_with_shape_hints() {
        let l = labels(&scene(), 480);
        let mut wanted = vec![true; PALETTE.len()];
        wanted[palette_index("white").unwrap()] = false;
        let r = find_regions(&l, &wanted, 5);
        let by = |n: &str| r.iter().find(|x| name(x.color) == n).unwrap().clone();
        assert_eq!(by("brown").shape(), "round");
        assert_eq!(by("green").shape(), "thin");
        assert_eq!(by("orange").shape(), "box");
        let (text, data) = regions_text(&l, &r, (1000, 500), 10, Some(palette_index("white").unwrap()));
        assert!(text.starts_with("3 region(s): ") && text.contains("background white"), "{text}");
        assert!(text.contains("orange box @1150,520 30x30 centre 1165,535"), "screen coordinates: {text}");
        assert_eq!(data["total"], 3);
    }

    #[test]
    fn template_is_found_where_it_is() {
        // a noisy-looking texture so the match is unique
        let big = img(300, 200, |x, y| (((x * 7 + y * 13) % 251) as u8, ((x * x + y * 3) % 253) as u8, ((x * 5 + y * y) % 241) as u8));
        let (tx, ty, tw, th) = (173, 61, 40, 30);
        let mut tb = vec![0u8; tw * th * 4];
        for y in 0..th {
            for x in 0..tw {
                let s = ((ty + y) * big.w + tx + x) * 4;
                tb[(y * tw + x) * 4..(y * tw + x) * 4 + 4].copy_from_slice(&big.bgra[s..s + 4]);
            }
        }
        let tpl = Image { w: tw, h: th, bgra: tb };
        let m = match_template(&big, &tpl, 0.9, 3);
        assert_eq!(m.first().map(|f| (f.0, f.1)), Some((tx, ty)), "{m:?}");
        assert!(m[0].2 > 0.99);
        let none = match_template(&big, &img(40, 30, |x, y| ((x * 31) as u8, (y * 17) as u8, 90)), 0.9, 3);
        assert!(none.is_empty(), "{none:?}");
    }

    #[test]
    fn arguments() {
        assert_eq!(parse_grid(&json!({})).unwrap(), (8, 6));
        assert_eq!(parse_grid(&json!({"grid": "12x8"})).unwrap(), (12, 8));
        assert!(parse_grid(&json!({"grid": "30x8"})).is_err());
        assert!(parse_grid(&json!({"grid": "big"})).is_err());
        assert_eq!(palette_index(" Brown "), Some(palette_index("brown").unwrap()));
        assert!(palette_index("magenta-ish").is_none());
    }
}

#[cfg(test)]
mod detail_tests {
    use super::{detail, Image};

    #[test]
    fn flat_patches_have_no_detail() {
        let flat = Image { w: 20, h: 20, bgra: [30u8, 30, 30, 255].repeat(400) };
        assert!(detail(&flat) < 1.0);
        let mut px = Vec::new();
        for i in 0..400 {
            let v = if (i / 20 + i % 20) % 2 == 0 { 0u8 } else { 255 };
            px.extend_from_slice(&[v, v, v, 255]);
        }
        assert!(detail(&Image { w: 20, h: 20, bgra: px }) > 100.0);
    }
}
