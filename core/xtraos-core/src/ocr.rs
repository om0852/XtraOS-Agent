//! OCR for opaque windows (P2-M2, ADR 0024): Windows.Media.Ocr, built into Windows, offline.
//! Returns text lines with SCREEN coordinates (physical px), ready for click(x, y): the agent
//! gets a few lines of text instead of an image.
//!
//! Docs: OcrEngine https://learn.microsoft.com/uwp/api/windows.media.ocr.ocrengine
use std::time::Instant;

use serde_json::{json, Value};
use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Security::Cryptography::CryptographicBuffer;

use crate::input::ActError;

fn bad(m: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: m.into(),
        hint: "Check the tool schema in docs/04-mcp-tool-contract.md.",
    }
}

fn winrt(what: &str, e: windows::core::Error) -> ActError {
    ActError {
        code: "INTERNAL",
        message: format!("{what}: {e}"),
        hint: "",
    }
}

/// A recognised word: text + box in image pixels.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Union box of words: (x, y, w, h).
pub fn union(words: &[Word]) -> (f32, f32, f32, f32) {
    let x0 = words.iter().map(|w| w.x).fold(f32::MAX, f32::min);
    let y0 = words.iter().map(|w| w.y).fold(f32::MAX, f32::min);
    let x1 = words.iter().map(|w| w.x + w.w).fold(f32::MIN, f32::max);
    let y1 = words.iter().map(|w| w.y + w.h).fold(f32::MIN, f32::max);
    (x0, y0, x1 - x0, y1 - y0)
}

/// The shortest run of consecutive words whose text (joined by spaces) contains `q`
/// (lowercase). Lets `ocr(query)` point at "Start Game" inside a longer line.
pub fn match_span(words: &[Word], q: &str) -> Option<(usize, usize)> {
    let mut best: Option<(usize, usize)> = None;
    for i in 0..words.len() {
        let mut joined = String::new();
        for (j, w) in words.iter().enumerate().skip(i) {
            if j > i {
                joined.push(' ');
            }
            joined.push_str(&w.text.to_lowercase());
            if joined.contains(q) {
                if best.is_none_or(|(a, b)| j - i < b - a) {
                    best = Some((i, j));
                }
                break;
            }
        }
    }
    best
}

/// `"text" @cx,cy (x,y wxh)` with screen coordinates.
pub fn line(text: &str, b: (f32, f32, f32, f32), origin: (i32, i32)) -> String {
    let (x, y) = (origin.0 + b.0.round() as i32, origin.1 + b.1.round() as i32);
    let (w, h) = (b.2.round() as i32, b.3.round() as i32);
    format!(
        "\"{}\" @{},{} ({x},{y} {w}x{h})",
        crate::perceive::clip(text, 80),
        x + w / 2,
        y + h / 2
    )
}

fn engine(language: Option<&str>) -> Result<OcrEngine, ActError> {
    let e = match language {
        Some(tag) => {
            let lang = Language::CreateLanguage(&HSTRING::from(tag))
                .map_err(|_| bad(format!("'{tag}' is not a language tag (e.g. en-US)")))?;
            OcrEngine::TryCreateFromLanguage(&lang)
        }
        None => OcrEngine::TryCreateFromUserProfileLanguages(),
    };
    e.map_err(|_| ActError {
        code: "NEEDS_HUMAN",
        message: format!(
            "no Windows OCR language is installed{}",
            language.map(|l| format!(" for {l}")).unwrap_or_default()
        ),
        hint: "Ask the user to add one: Settings > Time & language > Language & region > (language) > Language options > Optical character recognition.",
    })
}

pub fn ocr(p: &Value) -> Result<Value, ActError> {
    let t0 = Instant::now();
    let query = p
        .get("query")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let max_lines = match p.get("max_lines") {
        None | Some(Value::Null) => 80,
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=500).contains(n))
            .ok_or_else(|| bad("'max_lines' must be 1..=500"))? as usize,
    };
    let language = p.get("language").and_then(Value::as_str);
    crate::perceive::com_init();
    let eng = engine(language)?;
    let (img, origin, window) = crate::capture::grab(p)?;
    let maxd = OcrEngine::MaxImageDimension().unwrap_or(10_000) as usize;
    if img.w > maxd || img.h > maxd {
        return Err(bad(format!(
            "the area is larger than the OCR limit ({maxd} px); pass a smaller region"
        )));
    }
    let lines = unsafe_free_recognise(&eng, &img)?;
    let lang = eng
        .RecognizerLanguage()
        .and_then(|l| l.LanguageTag())
        .map(|t| t.to_string())
        .unwrap_or_default();

    let mut out = Vec::new();
    let mut total = 0;
    for (text, words) in &lines {
        if words.is_empty() {
            continue;
        }
        match &query {
            Some(q) => {
                if let Some((i, j)) = match_span(words, q) {
                    total += 1;
                    if out.len() < max_lines {
                        out.push(line(text, union(&words[i..=j]), origin));
                    }
                }
            }
            None => {
                total += 1;
                if out.len() < max_lines {
                    out.push(line(text, union(words), origin));
                }
            }
        }
    }
    let head = format!(
        "{} ocr {lang}{}",
        window.clone().unwrap_or_else(|| "region".into()),
        query
            .as_ref()
            .map(|q| format!(" query \"{q}\""))
            .unwrap_or_default()
    );
    let footer = match &query {
        Some(_) => format!("-- {total} matching lines"),
        None if total > out.len() => format!(
            "-- {} lines (+{} more; raise max_lines)",
            out.len(),
            total - out.len()
        ),
        None => format!("-- {total} lines"),
    };
    let text = if out.is_empty() {
        format!("{head}\n{footer}")
    } else {
        format!("{head}\n{}\n{footer}", out.join("\n"))
    };
    Ok(json!({
        "text": text,
        "lines": total,
        "language": lang,
        "window": window,
        "ms": t0.elapsed().as_millis() as u64,
    }))
}

/// Runs the engine on a BGRA image: [(line text, words)].
fn unsafe_free_recognise(
    eng: &OcrEngine,
    img: &crate::capture::Image,
) -> Result<Vec<(String, Vec<Word>)>, ActError> {
    let buf = CryptographicBuffer::CreateFromByteArray(&img.bgra)
        .map_err(|e| winrt("image buffer", e))?;
    let bmp = SoftwareBitmap::CreateCopyFromBuffer(
        &buf,
        BitmapPixelFormat::Bgra8,
        img.w as i32,
        img.h as i32,
    )
    .map_err(|e| winrt("bitmap", e))?;
    let result = eng
        .RecognizeAsync(&bmp)
        .and_then(|op| op.join())
        .map_err(|e| winrt("recognition", e))?;
    let mut out = Vec::new();
    for l in result.Lines().map_err(|e| winrt("lines", e))? {
        let text = l.Text().map(|t| t.to_string()).unwrap_or_default();
        let mut words = Vec::new();
        if let Ok(ws) = l.Words() {
            for w in ws {
                if let (Ok(t), Ok(r)) = (w.Text(), w.BoundingRect()) {
                    words.push(Word {
                        text: t.to_string(),
                        x: r.X,
                        y: r.Y,
                        w: r.Width,
                        h: r.Height,
                    });
                }
            }
        }
        out.push((text, words));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(t: &str, x: f32) -> Word {
        Word {
            text: t.into(),
            x,
            y: 10.0,
            w: 40.0,
            h: 20.0,
        }
    }

    #[test]
    fn spans_and_boxes() {
        let ws = vec![
            w("Press", 0.0),
            w("Start", 50.0),
            w("Game", 100.0),
            w("now", 150.0),
        ];
        assert_eq!(match_span(&ws, "start game"), Some((1, 2)));
        assert_eq!(match_span(&ws, "game"), Some((2, 2)));
        assert_eq!(match_span(&ws, "tart ga"), Some((1, 2)));
        assert_eq!(match_span(&ws, "quit"), None);
        assert_eq!(union(&ws[1..=2]), (50.0, 10.0, 90.0, 20.0));
        assert_eq!(
            line("Start Game", union(&ws[1..=2]), (100, 200)),
            "\"Start Game\" @195,220 (150,210 90x20)"
        );
    }

    #[test]
    fn arguments_are_checked_before_capture() {
        assert_eq!(
            ocr(&json!({"max_lines": 0})).unwrap_err().code,
            "INVALID_ARGS"
        );
        assert_eq!(
            ocr(&json!({"region": {"x": 0, "y": 0, "w": 0, "h": 5}}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
    }
}
