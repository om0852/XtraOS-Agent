//! `volume`: the Windows master volume of the default speakers, exactly, in the background
//! (Core Audio IAudioEndpointVolume; no keys, no mouse). Levels are 0–100 like the Windows
//! volume flyout. Media/volume keys also exist for `press_keys` (VolumeUp/Down/Mute,
//! PlayPause, NextTrack, PrevTrack, MediaStop).
use serde_json::{json, Value};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

use crate::input::ActError;

fn bad(m: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: m.into(),
        hint: "volume {op: get | set (value 0-100) | change (by -100..100) | mute | unmute}",
    }
}

fn audio_err(e: windows::core::Error) -> ActError {
    ActError {
        code: "INTERNAL",
        message: format!("speaker volume unavailable: {e}"),
        hint: "No default playback device? Check Windows sound settings.",
    }
}

fn endpoint() -> Result<IAudioEndpointVolume, ActError> {
    crate::perceive::com_init();
    unsafe {
        let en: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(audio_err)?;
        let dev = en
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(audio_err)?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None)
            .map_err(audio_err)
    }
}

fn read(ep: &IAudioEndpointVolume) -> Result<(u32, bool), ActError> {
    unsafe {
        let v = ep.GetMasterVolumeLevelScalar().map_err(audio_err)?;
        let m = ep.GetMute().map_err(audio_err)?.as_bool();
        Ok(((v * 100.0).round() as u32, m))
    }
}

/// New level for an op (pure; unit-tested).
pub fn target(op: &str, current: u32, p: &Value) -> Result<Option<u32>, ActError> {
    let num = |k: &str| p.get(k).and_then(Value::as_i64);
    Ok(match op {
        "get" | "mute" | "unmute" => None,
        "set" => {
            let v = num("value").ok_or_else(|| bad("set needs 'value' 0-100"))?;
            if !(0..=100).contains(&v) {
                return Err(bad("'value' must be 0-100"));
            }
            Some(v as u32)
        }
        "change" => {
            let d = num("by").ok_or_else(|| bad("change needs 'by' (e.g. -10)"))?;
            if !(-100..=100).contains(&d) {
                return Err(bad("'by' must be -100..100"));
            }
            Some((current as i64 + d).clamp(0, 100) as u32)
        }
        o => return Err(bad(format!("unknown op '{o}'"))),
    })
}

/// The `volume` method.
pub fn run(p: &Value) -> Result<Value, ActError> {
    crate::safety::check()?;
    let op = p.get("op").and_then(Value::as_str).unwrap_or("get");
    let ep = endpoint()?;
    let (before, muted_before) = read(&ep)?;
    let to = target(op, before, p)?;
    let ctx = windows::core::GUID::zeroed();
    unsafe {
        if let Some(v) = to {
            ep.SetMasterVolumeLevelScalar(v as f32 / 100.0, &ctx)
                .map_err(audio_err)?;
        }
        match op {
            "mute" => ep.SetMute(true, &ctx).map_err(audio_err)?,
            "unmute" => ep.SetMute(false, &ctx).map_err(audio_err)?,
            _ => {}
        }
    }
    let (after, muted) = read(&ep)?;
    let text = if op == "get" {
        format!(
            "system volume {after}{}",
            if muted { " (muted)" } else { "" }
        )
    } else {
        format!(
            "ok system volume {before}{} → {after}{}",
            if muted_before { " (muted)" } else { "" },
            if muted { " (muted)" } else { "" }
        )
    };
    Ok(json!({"text": text, "volume": after, "muted": muted, "before": before}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets() {
        assert_eq!(target("get", 58, &json!({})).unwrap(), None);
        assert_eq!(target("set", 58, &json!({"value": 67})).unwrap(), Some(67));
        assert_eq!(target("change", 58, &json!({"by": -10})).unwrap(), Some(48));
        assert_eq!(
            target("change", 5, &json!({"by": -10})).unwrap(),
            Some(0),
            "clamped"
        );
        assert_eq!(target("change", 95, &json!({"by": 10})).unwrap(), Some(100));
        assert!(target("set", 1, &json!({"value": 101})).is_err());
        assert!(target("set", 1, &json!({})).is_err());
        assert!(target("louder", 1, &json!({})).is_err());
    }
}
