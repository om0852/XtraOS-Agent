//! `system`: common Windows controls (ADR 0039), each through the OS API, in the background.
//!   battery                      read (percent, charging, time left)
//!   brightness [value 0-100]     read / set the built-in display (WMI WmiMonitorBrightness)
//!   wifi | bluetooth [state on|off]   read / switch the radio (Windows.Devices.Radios)
//!   lock                         lock the PC (LockWorkStation)
//!   sleep                        sleep (SetSuspendState)
//! Switching radios, lock and sleep are policy class system_settings (the user is asked).
//! Deliberately not here: shutdown/restart/sign-out, firewall, Defender, accounts, network
//! configuration, registry — outside XtraOS's scope (docs/05).
use serde_json::{json, Value};
use windows::core::{BSTR, PCWSTR};
use windows::Devices::Radios::{Radio, RadioAccessStatus, RadioKind, RadioState};
use windows::Win32::System::Com::{
    CoCreateInstance, CoSetProxyBlanket, CLSCTX_INPROC_SERVER, EOAC_NONE, RPC_C_AUTHN_LEVEL_CALL,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_UI1, VT_UI4};
use windows::Win32::System::Wmi::{
    IWbemClassObject, IWbemLocator, IWbemServices, WbemLocator, WBEM_FLAG_FORWARD_ONLY,
    WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_INFINITE,
};

use crate::input::ActError;

fn bad(m: impl Into<String>) -> ActError {
    ActError {
        code: "INVALID_ARGS",
        message: m.into(),
        hint: "system {op: battery | brightness [value] | wifi [state] | bluetooth [state] | lock | sleep}",
    }
}

fn os_err(what: &str, e: impl std::fmt::Display) -> ActError {
    ActError {
        code: "INTERNAL",
        message: format!("{what}: {e}"),
        hint: "",
    }
}

/// Whether an op changes something, and if so whether it needs the user's OK (pure).
pub fn change_kind(p: &Value) -> Option<&'static str> {
    match p.get("op").and_then(Value::as_str).unwrap_or("") {
        "brightness" if p.get("value").is_some() => Some("input"),
        "wifi" | "bluetooth" if p.get("state").is_some() => Some("system_settings"),
        "lock" | "sleep" => Some("system_settings"),
        _ => None,
    }
}

// ---------------------------------------------------------------------------------------------
// battery

#[repr(C)]
#[derive(Default)]
struct PowerStatus {
    ac: u8,
    flag: u8,
    percent: u8,
    saver: u8,
    life: u32,
    full: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetSystemPowerStatus(s: *mut PowerStatus) -> i32;
}
#[link(name = "user32")]
extern "system" {
    fn LockWorkStation() -> i32;
}
#[link(name = "powrprof")]
extern "system" {
    fn SetSuspendState(hibernate: u8, force: u8, wake_events_disabled: u8) -> u8;
}

fn battery() -> Result<Value, ActError> {
    let mut s = PowerStatus::default();
    if unsafe { GetSystemPowerStatus(&mut s) } == 0 {
        return Err(os_err("battery", std::io::Error::last_os_error()));
    }
    if s.flag == 128 || s.percent == 255 {
        return Ok(json!({"text": "no battery (desktop power)", "battery": false}));
    }
    let charging = s.ac == 1;
    let left = (s.life != u32::MAX).then_some(s.life / 60);
    Ok(json!({
        "text": format!("battery {}%{}{}{}", s.percent,
            if charging { ", plugged in" } else { ", on battery" },
            left.map(|m| format!(", about {} h {} min left", m / 60, m % 60)).unwrap_or_default(),
            if s.saver == 1 { ", battery saver on" } else { "" }),
        "battery": true, "percent": s.percent, "plugged_in": charging, "minutes_left": left,
        "battery_saver": s.saver == 1,
    }))
}

// ---------------------------------------------------------------------------------------------
// brightness (WMI root\WMI)

fn wmi() -> Result<IWbemServices, ActError> {
    crate::perceive::com_init();
    unsafe {
        let loc: IWbemLocator = CoCreateInstance(&WbemLocator, None, CLSCTX_INPROC_SERVER)
            .map_err(|e| os_err("WMI", e))?;
        let svc = loc
            .ConnectServer(
                &BSTR::from("ROOT\\WMI"),
                &BSTR::new(),
                &BSTR::new(),
                &BSTR::new(),
                0,
                &BSTR::new(),
                None,
            )
            .map_err(|e| os_err("WMI root\\WMI", e))?;
        CoSetProxyBlanket(
            &svc,
            10,
            0,
            PCWSTR::null(),
            RPC_C_AUTHN_LEVEL_CALL,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOAC_NONE,
        )
        .map_err(|e| os_err("WMI security", e))?;
        Ok(svc)
    }
}

fn first(svc: &IWbemServices, query: &str) -> Result<Option<IWbemClassObject>, ActError> {
    unsafe {
        let en = svc
            .ExecQuery(
                &BSTR::from("WQL"),
                &BSTR::from(query),
                WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY,
                None,
            )
            .map_err(|e| os_err("WMI query", e))?;
        let mut objs = [None];
        let mut n = 0u32;
        let _ = en.Next(WBEM_INFINITE, &mut objs, &mut n);
        Ok(if n == 0 { None } else { objs[0].take() })
    }
}

fn prop_u32(o: &IWbemClassObject, name: &str) -> Option<u32> {
    let w: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let mut v = VARIANT::default();
    unsafe {
        o.Get(PCWSTR(w.as_ptr()), 0, &mut v, None, None).ok()?;
        let inner = &v.Anonymous.Anonymous;
        let out = match inner.vt {
            VT_UI1 => Some(inner.Anonymous.bVal as u32),
            VT_UI4 => Some(inner.Anonymous.ulVal),
            _ => (inner.Anonymous.lVal >= 0).then_some(inner.Anonymous.lVal as u32),
        };
        let _ = VariantClear(&mut v);
        out
    }
}

fn no_panel() -> ActError {
    ActError {
        code: "ELEMENT_NOT_FOUND",
        message: "this display's brightness cannot be controlled by Windows (no built-in panel)"
            .into(),
        hint: "External monitors are set with their own buttons.",
    }
}

fn brightness(value: Option<u32>) -> Result<Value, ActError> {
    let svc = wmi()?;
    let cur = first(
        &svc,
        "SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE",
    )?
    .ok_or_else(no_panel)?;
    let before = prop_u32(&cur, "CurrentBrightness").unwrap_or(0);
    let Some(v) = value else {
        return Ok(json!({"text": format!("brightness {before}"), "brightness": before}));
    };
    unsafe {
        let m = first(
            &svc,
            "SELECT * FROM WmiMonitorBrightnessMethods WHERE Active=TRUE",
        )?
        .ok_or_else(no_panel)?;
        let path = {
            let w: Vec<u16> = "__PATH".encode_utf16().chain([0]).collect();
            let mut p = VARIANT::default();
            m.Get(PCWSTR(w.as_ptr()), 0, &mut p, None, None)
                .map_err(|e| os_err("WMI path", e))?;
            let s = (*p.Anonymous.Anonymous.Anonymous.bstrVal).clone();
            let _ = VariantClear(&mut p);
            s
        };
        let mut class = None;
        svc.GetObject(
            &BSTR::from("WmiMonitorBrightnessMethods"),
            Default::default(),
            None,
            Some(&mut class),
            None,
        )
        .map_err(|e| os_err("WMI class", e))?;
        let class = class.ok_or_else(|| os_err("WMI class", "missing"))?;
        let mut in_def = None;
        let name: Vec<u16> = "WmiSetBrightness".encode_utf16().chain([0]).collect();
        class
            .GetMethod(PCWSTR(name.as_ptr()), 0, &mut in_def, std::ptr::null_mut())
            .map_err(|e| os_err("WMI method", e))?;
        let args = in_def
            .ok_or_else(|| os_err("WMI method", "no parameters"))?
            .SpawnInstance(0)
            .map_err(|e| os_err("WMI params", e))?;
        let put = |k: &str, vt, val: u32| -> Result<(), ActError> {
            let w: Vec<u16> = k.encode_utf16().chain([0]).collect();
            let mut x = VARIANT::default();
            (*x.Anonymous.Anonymous).vt = vt;
            if vt == VT_UI1 {
                (*x.Anonymous.Anonymous).Anonymous.bVal = val as u8;
            } else {
                (*x.Anonymous.Anonymous).Anonymous.ulVal = val;
            }
            args.Put(PCWSTR(w.as_ptr()), 0, &x, 0)
                .map_err(|e| os_err("WMI params", e))
        };
        // The schema types are uint32 Timeout and uint8 Brightness; WMI accepts them as VT_I4.
        put("Timeout", windows::Win32::System::Variant::VT_I4, 0)?;
        put("Brightness", windows::Win32::System::Variant::VT_I4, v)?;
        svc.ExecMethod(
            &path,
            &BSTR::from("WmiSetBrightness"),
            Default::default(),
            None,
            &args,
            None,
            None,
        )
        .map_err(|e| os_err("set brightness", e))?;
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    let after = first(
        &svc,
        "SELECT CurrentBrightness FROM WmiMonitorBrightness WHERE Active=TRUE",
    )?
    .and_then(|o| prop_u32(&o, "CurrentBrightness"))
    .unwrap_or(v);
    Ok(
        json!({"text": format!("ok brightness {before} → {after}"), "brightness": after, "before": before}),
    )
}

// ---------------------------------------------------------------------------------------------
// radios

fn radio(kind: RadioKind, label: &str, state: Option<&str>) -> Result<Value, ActError> {
    let want = match state {
        None => None,
        Some("on") => Some(RadioState::On),
        Some("off") => Some(RadioState::Off),
        Some(o) => return Err(bad(format!("state '{o}': on or off"))),
    };
    if want.is_some() {
        let a = Radio::RequestAccessAsync()
            .and_then(|op| op.join())
            .map_err(|e| os_err("radio access", e))?;
        if a != RadioAccessStatus::Allowed {
            return Err(ActError {
                code: "NEEDS_HUMAN",
                message: format!("Windows does not let apps switch {label} (access {a:?})"),
                hint: "Windows Settings > Privacy & security > Radios: allow desktop apps, or switch it yourself.",
            });
        }
    }
    let radios = Radio::GetRadiosAsync()
        .and_then(|op| op.join())
        .map_err(|e| os_err("radios", e))?;
    let r = radios
        .into_iter()
        .find(|r| r.Kind().ok() == Some(kind))
        .ok_or_else(|| ActError {
            code: "ELEMENT_NOT_FOUND",
            message: format!("no {label} radio on this PC"),
            hint: "",
        })?;
    let name = |s: RadioState| match s {
        RadioState::On => "on",
        RadioState::Off => "off",
        RadioState::Disabled => "disabled (turned off in hardware or by airplane mode)",
        _ => "unknown",
    };
    let before = r.State().map_err(|e| os_err(label, e))?;
    let Some(w) = want else {
        return Ok(json!({"text": format!("{label} {}", name(before)), "state": name(before)}));
    };
    let a = r
        .SetStateAsync(w)
        .and_then(|op| op.join())
        .map_err(|e| os_err(label, e))?;
    if a != RadioAccessStatus::Allowed {
        return Err(ActError {
            code: "NEEDS_HUMAN",
            message: format!("Windows refused to switch {label} ({a:?})"),
            hint: "Switch it in the quick settings yourself.",
        });
    }
    let after = r.State().unwrap_or(w);
    Ok(
        json!({"text": format!("ok {label} {} → {}", name(before), name(after)), "state": name(after)}),
    )
}

/// The `system` method.
pub fn run(p: &Value) -> Result<Value, ActError> {
    crate::safety::check()?;
    let op = p.get("op").and_then(Value::as_str).unwrap_or("");
    match op {
        "battery" => battery(),
        "brightness" => {
            let v = match p.get("value") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    v.as_u64()
                        .filter(|n| *n <= 100)
                        .ok_or_else(|| bad("'value' must be 0-100"))? as u32,
                ),
            };
            brightness(v)
        }
        "wifi" => radio(
            RadioKind::WiFi,
            "Wi-Fi",
            p.get("state").and_then(Value::as_str),
        ),
        "bluetooth" => radio(
            RadioKind::Bluetooth,
            "Bluetooth",
            p.get("state").and_then(Value::as_str),
        ),
        "lock" => {
            if unsafe { LockWorkStation() } == 0 {
                return Err(os_err("lock", std::io::Error::last_os_error()));
            }
            Ok(
                json!({"text": "ok locked: the user must sign in again (agent actions now return NEEDS_HUMAN)"}),
            )
        }
        "sleep" => {
            // Reply first; the PC sleeps a moment later.
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(800));
                unsafe { SetSuspendState(0, 0, 0) };
            });
            Ok(json!({"text": "ok going to sleep"}))
        }
        o => Err(bad(format!("unknown op '{o}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_changes_and_what_asks() {
        assert_eq!(change_kind(&json!({"op": "battery"})), None);
        assert_eq!(change_kind(&json!({"op": "brightness"})), None);
        assert_eq!(
            change_kind(&json!({"op": "brightness", "value": 40})),
            Some("input")
        );
        assert_eq!(change_kind(&json!({"op": "wifi"})), None);
        assert_eq!(
            change_kind(&json!({"op": "wifi", "state": "off"})),
            Some("system_settings")
        );
        assert_eq!(
            change_kind(&json!({"op": "bluetooth", "state": "on"})),
            Some("system_settings")
        );
        assert_eq!(change_kind(&json!({"op": "lock"})), Some("system_settings"));
        assert_eq!(
            change_kind(&json!({"op": "sleep"})),
            Some("system_settings")
        );
        assert_eq!(
            run(&json!({"op": "shutdown"})).unwrap_err().code,
            "INVALID_ARGS",
            "not offered"
        );
        assert_eq!(
            run(&json!({"op": "brightness", "value": 300}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
    }
}
