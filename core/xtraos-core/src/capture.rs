//! Screenshots (Phase 1 M4, ADR 0015): a window via PrintWindow(PW_RENDERFULLCONTENT) cropped
//! to its visible frame (works when covered), or a screen region via BitBlt. Box-filter
//! downscale, PNG. All coordinates are physical pixels (ADR 0004); the reply carries the
//! mapping `screen = origin + px / scale` so the agent never does DPI maths.
//!
//! Docs:
//! PrintWindow https://learn.microsoft.com/windows/win32/api/winuser/nf-winuser-printwindow
//! BitBlt      https://learn.microsoft.com/windows/win32/api/wingdi/nf-wingdi-bitblt
//! GetDIBits   https://learn.microsoft.com/windows/win32/api/wingdi/nf-wingdi-getdibits
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use serde_json::{json, Value};
use windows::Win32::Foundation::HWND as WHWND;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC,
    SRCCOPY,
};
use windows::Win32::Storage::Xps::{PrintWindow, PRINT_WINDOW_FLAGS};

use crate::input::{ActError, HWND};
use crate::winmgr::{self, RECT};

const PW_RENDERFULLCONTENT: u32 = 2;
const MAX_SIDE: i32 = 16_384;

fn bad(message: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: message.into(),
        hint: "Check the tool schema in docs/04-mcp-tool-contract.md.",
    }
}

fn internal(message: impl Into<String>) -> ActError {
    ActError {
        code: "INTERNAL",
        message: message.into(),
        hint: "",
    }
}

/// BGRA pixels, top-down rows.
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub bgra: Vec<u8>,
}

/// Memory DC + bitmap of a given size; everything is released on drop.
struct Canvas {
    screen: HDC,
    mem: HDC,
    bmp: HBITMAP,
    old: windows::Win32::Graphics::Gdi::HGDIOBJ,
    w: i32,
    h: i32,
}

impl Canvas {
    fn new(w: i32, h: i32) -> Result<Self, ActError> {
        unsafe {
            let screen = GetDC(None);
            if screen.is_invalid() {
                return Err(internal("GetDC failed"));
            }
            let mem = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, w, h);
            if mem.is_invalid() || bmp.is_invalid() {
                ReleaseDC(None, screen);
                return Err(internal("could not create a capture bitmap"));
            }
            let old = SelectObject(mem, bmp.into());
            Ok(Canvas {
                screen,
                mem,
                bmp,
                old,
                w,
                h,
            })
        }
    }

    fn pixels(&self) -> Result<Image, ActError> {
        let mut bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: self.w,
                biHeight: -self.h, // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut buf = vec![0u8; self.w as usize * self.h as usize * 4];
        let lines = unsafe {
            GetDIBits(
                self.mem,
                self.bmp,
                0,
                self.h as u32,
                Some(buf.as_mut_ptr().cast()),
                &mut bmi,
                DIB_RGB_COLORS,
            )
        };
        if lines != self.h {
            return Err(internal("GetDIBits failed"));
        }
        Ok(Image {
            w: self.w as usize,
            h: self.h as usize,
            bgra: buf,
        })
    }
}

impl Drop for Canvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem, self.old);
            let _ = DeleteObject(self.bmp.into());
            let _ = DeleteDC(self.mem);
            ReleaseDC(None, self.screen);
        }
    }
}

fn grab_screen(r: RECT) -> Result<Image, ActError> {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let c = Canvas::new(w, h)?;
    unsafe { BitBlt(c.mem, 0, 0, w, h, Some(c.screen), r.left, r.top, SRCCOPY) }
        .map_err(|e| internal(format!("BitBlt failed: {e}")))?;
    c.pixels()
}

/// The window's own rendering (also when covered), cropped to the visible frame.
fn grab_window(h: HWND, frame: RECT) -> Result<Image, ActError> {
    let wr = winmgr::window_rect(h);
    let (w, hh) = (wr.right - wr.left, wr.bottom - wr.top);
    let c = Canvas::new(w, hh)?;
    let ok = unsafe { PrintWindow(WHWND(h), c.mem, PRINT_WINDOW_FLAGS(PW_RENDERFULLCONTENT)) };
    if !ok.as_bool() {
        drop(c);
        return grab_screen(frame); // fall back to what is on screen
    }
    let full = c.pixels()?;
    Ok(crop(
        &full,
        (frame.left - wr.left) as usize,
        (frame.top - wr.top) as usize,
        (frame.right - frame.left) as usize,
        (frame.bottom - frame.top) as usize,
    ))
}

pub fn crop(img: &Image, x: usize, y: usize, w: usize, h: usize) -> Image {
    let w = w.min(img.w.saturating_sub(x));
    let h = h.min(img.h.saturating_sub(y));
    let mut out = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        let s = (row * img.w + x) * 4;
        out.extend_from_slice(&img.bgra[s..s + w * 4]);
    }
    Image { w, h, bgra: out }
}

/// Box-filter downscale to round(w*scale) x round(h*scale); returns RGB bytes.
pub fn downscale_rgb(img: &Image, scale: f64) -> (usize, usize, Vec<u8>) {
    let ow = ((img.w as f64 * scale).round() as usize).max(1);
    let oh = ((img.h as f64 * scale).round() as usize).max(1);
    let mut out = Vec::with_capacity(ow * oh * 3);
    for oy in 0..oh {
        let y0 = oy * img.h / oh;
        let y1 = ((oy + 1) * img.h / oh).max(y0 + 1);
        for ox in 0..ow {
            let x0 = ox * img.w / ow;
            let x1 = ((ox + 1) * img.w / ow).max(x0 + 1);
            let (mut b, mut g, mut r, mut n) = (0u32, 0u32, 0u32, 0u32);
            for y in y0..y1 {
                let row = y * img.w * 4;
                for x in x0..x1 {
                    let i = row + x * 4;
                    b += img.bgra[i] as u32;
                    g += img.bgra[i + 1] as u32;
                    r += img.bgra[i + 2] as u32;
                    n += 1;
                }
            }
            out.extend_from_slice(&[(r / n) as u8, (g / n) as u8, (b / n) as u8]);
        }
    }
    (ow, oh, out)
}

pub fn png_rgb(w: usize, h: usize, rgb: &[u8]) -> Result<Vec<u8>, ActError> {
    let mut buf = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut buf, w as u32, h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut wr = enc
            .write_header()
            .map_err(|e| internal(format!("png: {e}")))?;
        wr.write_image_data(rgb)
            .map_err(|e| internal(format!("png: {e}")))?;
    }
    Ok(buf)
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16
            | (*c.get(1).unwrap_or(&0) as u32) << 8
            | *c.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

pub fn screenshot(p: &Value) -> Result<Value, ActError> {
    let scale = match p.get("scale") {
        None | Some(Value::Null) => 0.5,
        Some(v) => v
            .as_f64()
            .filter(|s| (0.1..=1.0).contains(s))
            .ok_or_else(|| bad("'scale' must be 0.1..=1"))?,
    };
    let (img, origin, window) = grab(p)?;
    let (w, h, rgb) = downscale_rgb(&img, scale);
    let png = png_rgb(w, h, &rgb)?;
    Ok(json!({
        "png_base64": base64(&png),
        "width": w,
        "height": h,
        "origin": [origin.0, origin.1],
        "scale": scale,
        "window": window,
        "bytes": png.len(),
    }))
}

/// Captures `window` (default foreground; also when covered) or `region` at full resolution:
/// (BGRA image, screen origin of its top-left pixel, window ref). Shared by screenshot and ocr.
/// A capture: image, screen origin of its top-left pixel, window ref (None for a region).
pub type Grab = (Image, (i32, i32), Option<String>);

pub fn grab(p: &Value) -> Result<Grab, ActError> {
    Ok(if let Some(r) = p.get("region").filter(|v| !v.is_null()) {
        let g = |k: &str| {
            r.get(k)
                .and_then(Value::as_i64)
                .map(|v| v as i32)
                .ok_or_else(|| bad("'region' is {x, y, w, h} (integers)"))
        };
        let (x, y, w, h) = (g("x")?, g("y")?, g("w")?, g("h")?);
        if w <= 0 || h <= 0 || w > MAX_SIDE || h > MAX_SIDE {
            return Err(bad("region size must be 1..=16384"));
        }
        let rect = RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        (grab_screen(rect)?, (x, y), None)
    } else {
        let h = match p.get("window") {
            None | Some(Value::Null) => {
                let fg = winmgr::foreground_root();
                if fg.is_null() {
                    return Err(ActError {
                        code: "WINDOW_NOT_FOUND",
                        message: "no foreground window".into(),
                        hint: "Pass 'window' or 'region'.",
                    });
                }
                fg
            }
            Some(v) => winmgr::resolve_window(v)?,
        };
        if winmgr::is_minimized(h) {
            return Err(ActError {
                code: "INVALID_ARGS",
                message: "the window is minimized; nothing to capture".into(),
                hint: "Restore it first: window_bounds(state: \"restore\").",
            });
        }
        let f = winmgr::frame(h);
        if f.right <= f.left || f.bottom <= f.top {
            return Err(internal("the window has no visible area"));
        }
        (
            grab_window(h, f)?,
            (f.left, f.top),
            Some(winmgr::window_ref(h)),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn downscale_averages_blocks() {
        // 2x2 BGRA: red, blue / blue, red  -> 1x1 average purple
        let img = Image {
            w: 2,
            h: 2,
            bgra: vec![
                0, 0, 200, 255, 200, 0, 0, 255, 200, 0, 0, 255, 0, 0, 200, 255,
            ],
        };
        let (w, h, rgb) = downscale_rgb(&img, 0.5);
        assert_eq!((w, h), (1, 1));
        assert_eq!(rgb, vec![100, 0, 100]);
    }

    #[test]
    fn crop_and_png_roundtrip() {
        let img = Image {
            w: 3,
            h: 2,
            bgra: (0..24).map(|i| i as u8).collect(),
        };
        let c = crop(&img, 1, 1, 5, 5);
        assert_eq!((c.w, c.h), (2, 1));
        assert_eq!(&c.bgra[..4], &[16, 17, 18, 19]);
        let (w, h, rgb) = downscale_rgb(&c, 1.0);
        let png = png_rgb(w, h, &rgb).unwrap();
        let dec = png::Decoder::new(std::io::Cursor::new(png));
        let mut r = dec.read_info().unwrap();
        let mut buf = vec![0; r.output_buffer_size().unwrap()];
        let info = r.next_frame(&mut buf).unwrap();
        assert_eq!((info.width, info.height), (2, 1));
        assert_eq!(&buf[..3], &[18, 17, 16]);
    }
}
