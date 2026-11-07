//! `touch` and `pen` (P3-M3, ADR 0036): synthetic pointer input through
//! CreateSyntheticPointerDevice + InjectSyntheticPointerInput (Windows 10 1809+).
//! Same rules as mouse input: polite (waits for the user to be idle), the kill switch is
//! checked before every frame (contacts are lifted even then), never on an XtraOS or admin
//! window. Multi-finger OS gestures (3–4 finger swipes) are policy class system_settings.
//! Touch feedback is shown (POINTER_FEEDBACK_DEFAULT) so the user sees what happens.
use std::thread::sleep;
use std::time::Duration;

use serde_json::{json, Value};
use windows::Win32::Foundation::{POINT as WPOINT, RECT};
use windows::Win32::UI::Controls::{
    CreateSyntheticPointerDevice, DestroySyntheticPointerDevice, HSYNTHETICPOINTERDEVICE,
    POINTER_FEEDBACK_DEFAULT, POINTER_TYPE_INFO, POINTER_TYPE_INFO_0,
};
use windows::Win32::UI::Input::Pointer::{
    InjectSyntheticPointerInput, POINTER_FLAGS, POINTER_FLAG_DOWN, POINTER_FLAG_INCONTACT,
    POINTER_FLAG_INRANGE, POINTER_FLAG_UP, POINTER_FLAG_UPDATE, POINTER_INFO, POINTER_PEN_INFO,
    POINTER_TOUCH_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{PT_PEN, PT_TOUCH};

use crate::input::{self, ActError, POINT};

const FRAME_MS: u64 = 10;
const TOUCH_MASK_CONTACTAREA: u32 = 0x1;
const TOUCH_MASK_ORIENTATION: u32 = 0x2;
const TOUCH_MASK_PRESSURE: u32 = 0x4;
const PEN_MASK_PRESSURE: u32 = 0x1;

fn bad(m: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: m.into(),
        hint: "See touch / pen in docs/04-mcp-tool-contract.md.",
    }
}

fn int(p: &Value, k: &str) -> Option<i32> {
    p.get(k).and_then(Value::as_i64).map(|v| v as i32)
}

fn need(p: &Value, k: &str) -> Result<i32, ActError> {
    int(p, k).ok_or_else(|| bad(format!("'{k}' (screen pixels) is required")))
}

/// One gesture as frames: each frame = positions of the contacts (same count every frame).
pub fn touch_frames(p: &Value) -> Result<(Vec<Vec<POINT>>, u32), ActError> {
    let action = p.get("action").and_then(Value::as_str).unwrap_or("tap");
    let x = need(p, "x")?;
    let y = need(p, "y")?;
    let dur = p
        .get("duration_ms")
        .and_then(Value::as_u64)
        .unwrap_or(match action {
            "long_press" => 800,
            "tap" => 60,
            _ => 300,
        })
        .clamp(20, 10_000);
    let steps = (dur / FRAME_MS).max(2) as usize;
    let lerp =
        |a: i32, b: i32, i: usize| a + ((b - a) as f64 * i as f64 / steps as f64).round() as i32;
    Ok(match action {
        "tap" | "long_press" => (vec![vec![POINT { x, y }]; steps], 1),
        "drag" | "swipe" => {
            let (tx, ty) = (need(p, "to_x")?, need(p, "to_y")?);
            let fingers = if action == "swipe" {
                p.get("fingers").and_then(Value::as_u64).unwrap_or(1)
            } else {
                1
            };
            if !(1..=4).contains(&fingers) {
                return Err(bad("'fingers' must be 1..=4"));
            }
            let gap = 40;
            let offs: Vec<i32> = (0..fingers as i32)
                .map(|i| i * gap - (fingers as i32 - 1) * gap / 2)
                .collect();
            (
                (0..=steps)
                    .map(|i| {
                        offs.iter()
                            .map(|o| POINT {
                                x: lerp(x, tx, i) + o,
                                y: lerp(y, ty, i),
                            })
                            .collect()
                    })
                    .collect(),
                fingers as u32,
            )
        }
        "pinch" => {
            let scale = p.get("scale").and_then(Value::as_f64).unwrap_or(0.5);
            if !(0.1..=10.0).contains(&scale) {
                return Err(bad(
                    "'scale' must be 0.1..=10 (below 1 pinches in, above 1 spreads)",
                ));
            }
            let d0 = int(p, "distance").unwrap_or(200).clamp(20, 2000) as f64;
            let d1 = d0 * scale;
            (
                (0..=steps)
                    .map(|i| {
                        let d = d0 + (d1 - d0) * i as f64 / steps as f64;
                        let h = (d / 2.0).round() as i32;
                        vec![POINT { x: x - h, y }, POINT { x: x + h, y }]
                    })
                    .collect(),
                2,
            )
        }
        o => {
            return Err(bad(format!(
                "touch action '{o}': tap, long_press, drag, pinch, swipe"
            )))
        }
    })
}

/// Pen path frames: (point, pressure 0..1024) per frame.
pub fn pen_frames(p: &Value) -> Result<Vec<(POINT, u32)>, ActError> {
    let action = p.get("action").and_then(Value::as_str).unwrap_or("tap");
    let pressure = p
        .get("pressure")
        .and_then(Value::as_u64)
        .unwrap_or(512)
        .min(1024) as u32;
    match action {
        "tap" => {
            let pt = POINT {
                x: need(p, "x")?,
                y: need(p, "y")?,
            };
            Ok(vec![(pt, pressure); 6])
        }
        "draw" => {
            let pts = p
                .get("points")
                .and_then(Value::as_array)
                .ok_or_else(|| bad("draw needs 'points': [[x, y], [x, y, pressure?], ...]"))?;
            if pts.len() < 2 || pts.len() > 2000 {
                return Err(bad("'points' needs 2..=2000 points"));
            }
            let mut v: Vec<(POINT, u32)> = Vec::new();
            for q in pts {
                let a = q
                    .as_array()
                    .ok_or_else(|| bad("each point is [x, y] or [x, y, pressure]"))?;
                let g = |i: usize| a.get(i).and_then(Value::as_i64);
                let (Some(px), Some(py)) = (g(0), g(1)) else {
                    return Err(bad("each point is [x, y] or [x, y, pressure]"));
                };
                let pr = g(2).map(|n| n.clamp(0, 1024) as u32).unwrap_or(pressure);
                let pt = POINT {
                    x: px as i32,
                    y: py as i32,
                };
                // Fill long segments so the stroke is continuous (≤ 8 px per frame).
                if let Some(&(last, lp)) = v.last() {
                    let (dx, dy) = (pt.x - last.x, pt.y - last.y);
                    let n = ((dx.abs().max(dy.abs())) / 8).max(1);
                    for i in 1..n {
                        v.push((
                            POINT {
                                x: last.x + dx * i / n,
                                y: last.y + dy * i / n,
                            },
                            lp + ((pr as i64 - lp as i64) * i as i64 / n as i64) as u32,
                        ));
                    }
                }
                v.push((pt, pr));
            }
            Ok(v)
        }
        o => Err(bad(format!("pen action '{o}': tap, draw"))),
    }
}

fn check_point(pt: POINT) -> Result<(), ActError> {
    let h = crate::winmgr::root_at(pt);
    if crate::safety::is_own_window(h) {
        return Err(crate::safety::blocked(
            "that point is on an XtraOS window: only the user may touch it",
        ));
    }
    if !h.is_null() && crate::winmgr::is_elevated(h) {
        return Err(crate::winmgr::elevation_required("touch at this point"));
    }
    Ok(())
}

struct Device(HSYNTHETICPOINTERDEVICE);
impl Drop for Device {
    fn drop(&mut self) {
        unsafe { DestroySyntheticPointerDevice(self.0) };
    }
}

fn pinfo(
    kind: windows::Win32::UI::WindowsAndMessaging::POINTER_INPUT_TYPE,
    id: u32,
    pt: POINT,
    flags: POINTER_FLAGS,
) -> POINTER_INFO {
    POINTER_INFO {
        pointerType: kind,
        pointerId: id,
        pointerFlags: flags,
        ptPixelLocation: WPOINT { x: pt.x, y: pt.y },
        ..Default::default()
    }
}

fn inject(dev: &Device, frame: &[POINTER_TYPE_INFO]) -> Result<(), ActError> {
    input::mark_injected();
    unsafe { InjectSyntheticPointerInput(dev.0, frame) }.map_err(|e| ActError {
        code: "INTERNAL",
        message: format!("pointer injection failed: {e}"),
        hint: "Touch injection needs Windows 10 1809 or later and the interactive desktop.",
    })
}

fn touch_frame(contacts: &[POINT], flags: POINTER_FLAGS) -> Vec<POINTER_TYPE_INFO> {
    contacts
        .iter()
        .enumerate()
        .map(|(i, &pt)| POINTER_TYPE_INFO {
            r#type: PT_TOUCH,
            Anonymous: POINTER_TYPE_INFO_0 {
                touchInfo: POINTER_TOUCH_INFO {
                    pointerInfo: pinfo(PT_TOUCH, i as u32, pt, flags),
                    touchMask: TOUCH_MASK_CONTACTAREA
                        | TOUCH_MASK_ORIENTATION
                        | TOUCH_MASK_PRESSURE,
                    rcContact: RECT {
                        left: pt.x - 2,
                        top: pt.y - 2,
                        right: pt.x + 2,
                        bottom: pt.y + 2,
                    },
                    orientation: 90,
                    pressure: 32000,
                    ..Default::default()
                },
            },
        })
        .collect()
}

fn pen_frame(pt: POINT, pressure: u32, flags: POINTER_FLAGS) -> [POINTER_TYPE_INFO; 1] {
    [POINTER_TYPE_INFO {
        r#type: PT_PEN,
        Anonymous: POINTER_TYPE_INFO_0 {
            penInfo: POINTER_PEN_INFO {
                pointerInfo: pinfo(PT_PEN, 0, pt, flags),
                penMask: PEN_MASK_PRESSURE,
                pressure,
                ..Default::default()
            },
        },
    }]
}

const DOWN: POINTER_FLAGS =
    POINTER_FLAGS(POINTER_FLAG_DOWN.0 | POINTER_FLAG_INRANGE.0 | POINTER_FLAG_INCONTACT.0);
const MOVE: POINTER_FLAGS =
    POINTER_FLAGS(POINTER_FLAG_UPDATE.0 | POINTER_FLAG_INRANGE.0 | POINTER_FLAG_INCONTACT.0);

/// The `touch` and `pen` methods.
pub fn run(method: &str, p: &Value) -> Result<Value, ActError> {
    input::init_dpi();
    crate::safety::check()?;
    let polite = input::polite_args(p, false)?;
    let _g = input::action_lock();
    match method {
        "touch" => {
            let (frames, n) = touch_frames(p)?;
            for f in [frames.first(), frames.last()].into_iter().flatten() {
                for &pt in f {
                    check_point(pt)?;
                }
            }
            input::wait_user_idle(&polite)?;
            let dev = Device(
                unsafe {
                    CreateSyntheticPointerDevice(PT_TOUCH, n.max(1), POINTER_FEEDBACK_DEFAULT)
                }
                .map_err(|e| bad(format!("touch device: {e}")))?,
            );
            let last = frames.last().cloned().unwrap_or_default();
            let mut res = inject(&dev, &touch_frame(&frames[0], DOWN));
            if res.is_ok() {
                for f in &frames[1..] {
                    if crate::safety::stopped() {
                        break;
                    }
                    sleep(Duration::from_millis(FRAME_MS));
                    if let Err(e) = inject(&dev, &touch_frame(f, MOVE)) {
                        res = Err(e);
                        break;
                    }
                }
            }
            // Always lift the contacts, even after the kill switch or an error.
            let _ = inject(&dev, &touch_frame(&last, POINTER_FLAG_UP));
            res?;
            crate::safety::check()?;
            Ok(
                json!({"text": format!("ok touch {} ({} contact(s), {} frames)",
                p.get("action").and_then(Value::as_str).unwrap_or("tap"), n, frames.len()), "mode": "real"}),
            )
        }
        "pen" => {
            let frames = pen_frames(p)?;
            check_point(frames[0].0)?;
            check_point(frames[frames.len() - 1].0)?;
            input::wait_user_idle(&polite)?;
            let dev = Device(
                unsafe { CreateSyntheticPointerDevice(PT_PEN, 1, POINTER_FEEDBACK_DEFAULT) }
                    .map_err(|e| bad(format!("pen device: {e}")))?,
            );
            let (lp, lpr) = frames[frames.len() - 1];
            let mut res = inject(&dev, &pen_frame(frames[0].0, frames[0].1, DOWN));
            if res.is_ok() {
                for &(pt, pr) in &frames[1..] {
                    if crate::safety::stopped() {
                        break;
                    }
                    sleep(Duration::from_millis(FRAME_MS));
                    if let Err(e) = inject(&dev, &pen_frame(pt, pr, MOVE)) {
                        res = Err(e);
                        break;
                    }
                }
            }
            let _ = inject(&dev, &pen_frame(lp, lpr, POINTER_FLAG_UP));
            res?;
            crate::safety::check()?;
            Ok(json!({"text": format!("ok pen {} ({} frames)",
                p.get("action").and_then(Value::as_str).unwrap_or("tap"), frames.len()), "mode": "real"}))
        }
        m => Err(bad(format!("'{m}' is not a pointer method"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gesture_frames() {
        let (f, n) = touch_frames(&json!({"action": "tap", "x": 10, "y": 20})).unwrap();
        assert_eq!(n, 1);
        assert!(f.iter().all(|c| c == &vec![POINT { x: 10, y: 20 }]));
        let (f, _) =
            touch_frames(&json!({"action": "drag", "x": 0, "y": 0, "to_x": 100, "to_y": 50}))
                .unwrap();
        assert_eq!(f.first().unwrap()[0], POINT { x: 0, y: 0 });
        assert_eq!(f.last().unwrap()[0], POINT { x: 100, y: 50 });
        let (f, n) = touch_frames(
            &json!({"action": "pinch", "x": 500, "y": 300, "distance": 200, "scale": 0.5}),
        )
        .unwrap();
        assert_eq!(n, 2);
        assert_eq!(
            f[0],
            vec![POINT { x: 400, y: 300 }, POINT { x: 600, y: 300 }]
        );
        assert_eq!(
            f.last().unwrap(),
            &vec![POINT { x: 450, y: 300 }, POINT { x: 550, y: 300 }]
        );
        let (f, n) = touch_frames(
            &json!({"action": "swipe", "fingers": 3, "x": 500, "y": 500, "to_x": 500, "to_y": 300}),
        )
        .unwrap();
        assert_eq!(n, 3);
        assert_eq!(f[0].len(), 3);
        assert!(touch_frames(
            &json!({"action": "swipe", "fingers": 5, "x": 1, "y": 1, "to_x": 1, "to_y": 1})
        )
        .is_err());
        assert!(touch_frames(&json!({"action": "rotate", "x": 1, "y": 1})).is_err());
        assert!(
            touch_frames(&json!({"action": "tap"})).is_err(),
            "x/y required"
        );
    }

    #[test]
    fn pen_paths_are_continuous() {
        let f = pen_frames(&json!({"action": "draw", "points": [[0, 0], [80, 0, 1024]]})).unwrap();
        assert!(f.windows(2).all(|w| (w[1].0.x - w[0].0.x).abs() <= 8));
        assert_eq!(f.last().unwrap(), &(POINT { x: 80, y: 0 }, 1024));
        assert!(pen_frames(&json!({"action": "draw", "points": [[0, 0]]})).is_err());
        assert_eq!(
            pen_frames(&json!({"action": "tap", "x": 5, "y": 6, "pressure": 5000})).unwrap()[0].1,
            1024
        );
    }
}
