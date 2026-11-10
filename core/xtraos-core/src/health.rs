//! System health guard (ADR 0050): readings, a watchdog, warnings for agents and the user,
//! and a brake on the agent opening more while the PC is under strain.
//!
//! Every reading works without admin rights (spike in ADR 0050):
//! - CPU: GetSystemTimes deltas; RAM: GlobalMemoryStatusEx; disks: GetDiskFreeSpaceExW
//! - GPU: PDH `\GPU Engine(*engtype_3D)\Utilization Percentage` (sum over engines)
//! - temperatures: PDH `\Thermal Zone Information(*)\Temperature` (kelvin; zones ≤ 0 °C dropped)
//! - Wi-Fi: WLAN API current connection, signal quality only (no network name: location data)
//! - apps: Toolhelp process list (counts); working set per app for `system health` only
//!
//! XtraOS never kills processes or closes the user's windows here; it warns and, at a critical
//! level, asks before the agent opens even more.
#![allow(non_snake_case, clippy::upper_case_acronyms)]

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::{Mutex, Once};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::input::ActError;

// ---------------------------------------------------------------------------------------------
// FFI

type HANDLE = *mut c_void;

#[repr(C)]
#[derive(Default)]
struct MEMORYSTATUSEX {
    dwLength: u32,
    dwMemoryLoad: u32,
    ullTotalPhys: u64,
    ullAvailPhys: u64,
    ullTotalPageFile: u64,
    ullAvailPageFile: u64,
    ullTotalVirtual: u64,
    ullAvailVirtual: u64,
    ullAvailExtendedVirtual: u64,
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct FILETIME {
    lo: u32,
    hi: u32,
}

impl FILETIME {
    fn get(self) -> u64 {
        (u64::from(self.hi) << 32) | u64::from(self.lo)
    }
}

#[repr(C)]
struct PROCESSENTRY32W {
    dwSize: u32,
    cntUsage: u32,
    th32ProcessID: u32,
    th32DefaultHeapID: usize,
    th32ModuleID: u32,
    cntThreads: u32,
    th32ParentProcessID: u32,
    pcPriClassBase: i32,
    dwFlags: u32,
    szExeFile: [u16; 260],
}

#[repr(C)]
#[derive(Default)]
struct PROCESS_MEMORY_COUNTERS {
    cb: u32,
    PageFaultCount: u32,
    PeakWorkingSetSize: usize,
    WorkingSetSize: usize,
    QuotaPeakPagedPoolUsage: usize,
    QuotaPagedPoolUsage: usize,
    QuotaPeakNonPagedPoolUsage: usize,
    QuotaNonPagedPoolUsage: usize,
    PagefileUsage: usize,
    PeakPagefileUsage: usize,
}

/// PDH_FMT_COUNTERVALUE_ITEM_W: name pointer + {CStatus, (pad), double}.
#[repr(C)]
struct PdhItem {
    name: *mut u16,
    status: u32,
    _pad: u32,
    value: f64,
}

#[link(name = "kernel32")]
extern "system" {
    fn GlobalMemoryStatusEx(m: *mut MEMORYSTATUSEX) -> i32;
    fn GetSystemTimes(idle: *mut FILETIME, kernel: *mut FILETIME, user: *mut FILETIME) -> i32;
    fn GetDiskFreeSpaceExW(root: *const u16, avail: *mut u64, total: *mut u64, free: *mut u64) -> i32;
    fn GetLogicalDrives() -> u32;
    fn GetDriveTypeW(root: *const u16) -> u32;
    fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> HANDLE;
    fn Process32FirstW(h: HANDLE, e: *mut PROCESSENTRY32W) -> i32;
    fn Process32NextW(h: HANDLE, e: *mut PROCESSENTRY32W) -> i32;
    fn OpenProcess(access: u32, inherit: i32, pid: u32) -> HANDLE;
    fn CloseHandle(h: HANDLE) -> i32;
    fn K32GetProcessMemoryInfo(h: HANDLE, c: *mut PROCESS_MEMORY_COUNTERS, cb: u32) -> i32;
}

#[link(name = "pdh")]
extern "system" {
    fn PdhOpenQueryW(src: *const u16, user: usize, q: *mut isize) -> u32;
    fn PdhAddEnglishCounterW(q: isize, path: *const u16, user: usize, c: *mut isize) -> u32;
    fn PdhCollectQueryData(q: isize) -> u32;
    fn PdhGetFormattedCounterArrayW(c: isize, fmt: u32, size: *mut u32, count: *mut u32, items: *mut PdhItem) -> u32;
}

#[link(name = "wlanapi")]
extern "system" {
    fn WlanOpenHandle(ver: u32, res: *mut c_void, neg: *mut u32, h: *mut HANDLE) -> u32;
    fn WlanCloseHandle(h: HANDLE, res: *mut c_void) -> u32;
    fn WlanEnumInterfaces(h: HANDLE, res: *mut c_void, list: *mut *mut u8) -> u32;
    fn WlanQueryInterface(h: HANDLE, guid: *const u8, op: u32, res: *mut c_void, size: *mut u32, data: *mut *mut u8, vt: *mut u32) -> u32;
    fn WlanFreeMemory(p: *mut c_void);
}

const TH32CS_SNAPPROCESS: u32 = 0x2;
const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PDH_FMT_DOUBLE: u32 = 0x200;
const PDH_MORE_DATA: u32 = 0x8000_07D2;
const DRIVE_FIXED: u32 = 3;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ---------------------------------------------------------------------------------------------
// Readings

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    pub cpu_pct: Option<f64>,
    pub ram_used_gb: f64,
    pub ram_total_gb: f64,
    pub ram_pct: f64,
    pub gpu_pct: Option<f64>,
    /// (zone, °C), zones reporting ≤ 0 °C dropped.
    pub temps: Vec<(String, f64)>,
    /// (drive, free GB, total GB)
    pub disks: Vec<(String, f64, f64)>,
    /// Process count per executable name (lower case).
    pub procs: HashMap<String, u32>,
}

fn ram() -> (f64, f64, f64) {
    let mut m = MEMORYSTATUSEX {
        dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
        ..Default::default()
    };
    if unsafe { GlobalMemoryStatusEx(&mut m) } == 0 || m.ullTotalPhys == 0 {
        return (0.0, 0.0, 0.0);
    }
    let gb = |b: u64| b as f64 / 1_073_741_824.0;
    let used = m.ullTotalPhys - m.ullAvailPhys;
    (gb(used), gb(m.ullTotalPhys), used as f64 * 100.0 / m.ullTotalPhys as f64)
}

fn system_times() -> Option<(u64, u64)> {
    let (mut i, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    if unsafe { GetSystemTimes(&mut i, &mut k, &mut u) } == 0 {
        return None;
    }
    // kernel time includes idle time
    Some((i.get(), k.get() + u.get()))
}

/// CPU busy % between two (idle, total) samples (pure).
pub fn cpu_between(a: (u64, u64), b: (u64, u64)) -> Option<f64> {
    let idle = b.0.checked_sub(a.0)?;
    let total = b.1.checked_sub(a.1)?;
    (total > 0).then(|| (100.0 * (total.saturating_sub(idle)) as f64 / total as f64).clamp(0.0, 100.0))
}

fn fixed_disks() -> Vec<(String, f64, f64)> {
    let mask = unsafe { GetLogicalDrives() };
    let mut out = Vec::new();
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + i as u8) as char);
        let w = wide(&root);
        if unsafe { GetDriveTypeW(w.as_ptr()) } != DRIVE_FIXED {
            continue;
        }
        let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
        if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free) } != 0 && total > 0 {
            out.push((root[..2].to_string(), avail as f64 / 1_073_741_824.0, total as f64 / 1_073_741_824.0));
        }
    }
    out
}

/// (pid, exe name lower case) of every process.
fn processes() -> Vec<(u32, String)> {
    let mut out = Vec::new();
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap as isize == -1 {
            return out;
        }
        let mut e: PROCESSENTRY32W = std::mem::zeroed();
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut e);
        while ok != 0 {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(260);
            out.push((e.th32ProcessID, String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase()));
            ok = Process32NextW(snap, &mut e);
        }
        CloseHandle(snap);
    }
    out
}

fn counts(procs: &[(u32, String)]) -> HashMap<String, u32> {
    let mut m = HashMap::new();
    for (_, n) in procs {
        *m.entry(n.clone()).or_insert(0) += 1;
    }
    m
}

/// The apps using most memory: (app, processes, MB), top `n`.
fn top_apps(n: usize) -> Vec<(String, u32, u64)> {
    let mut by: HashMap<String, (u32, u64)> = HashMap::new();
    for (pid, name) in processes() {
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        let mut ws = 0u64;
        if !h.is_null() {
            let mut c = PROCESS_MEMORY_COUNTERS {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ..Default::default()
            };
            if unsafe { K32GetProcessMemoryInfo(h, &mut c, c.cb) } != 0 {
                ws = c.WorkingSetSize as u64;
            }
            unsafe { CloseHandle(h) };
        }
        let e = by.entry(name).or_insert((0, 0));
        e.0 += 1;
        e.1 += ws;
    }
    let mut v: Vec<(String, u32, u64)> = by
        .into_iter()
        .map(|(k, (c, b))| (k.trim_end_matches(".exe").to_string(), c, b / 1_048_576))
        .collect();
    v.sort_by(|a, b| b.2.cmp(&a.2));
    v.truncate(n);
    v
}

/// Wi-Fi signal quality (0–100) of the first connected interface; None: no Wi-Fi / not connected.
fn wifi_signal() -> Option<u32> {
    unsafe {
        let (mut neg, mut h) = (0u32, std::ptr::null_mut());
        if WlanOpenHandle(2, std::ptr::null_mut(), &mut neg, &mut h) != 0 {
            return None;
        }
        let mut out = None;
        let mut list: *mut u8 = std::ptr::null_mut();
        if WlanEnumInterfaces(h, std::ptr::null_mut(), &mut list) == 0 && !list.is_null() {
            let n = *(list as *const u32);
            // WLAN_INTERFACE_INFO_LIST: count, index, then items of 532 bytes:
            // GUID (16), description [u16; 256] (512), state u32 (connected = 1).
            for i in 0..n.min(8) as usize {
                let item = list.add(8 + i * 532);
                let state = *(item.add(528) as *const u32);
                if state != 1 {
                    continue;
                }
                let (mut size, mut data, mut vt) = (0u32, std::ptr::null_mut(), 0u32);
                // wlan_intf_opcode_current_connection = 7 → WLAN_CONNECTION_ATTRIBUTES;
                // wlanSignalQuality sits at byte 576 (see ADR 0050).
                if WlanQueryInterface(h, item, 7, std::ptr::null_mut(), &mut size, &mut data, &mut vt) == 0
                    && !data.is_null()
                {
                    if size as usize >= 580 {
                        out = Some((*(data.add(576) as *const u32)).min(100));
                    }
                    WlanFreeMemory(data as *mut c_void);
                }
                if out.is_some() {
                    break;
                }
            }
            WlanFreeMemory(list as *mut c_void);
        }
        WlanCloseHandle(h, std::ptr::null_mut());
        out
    }
}

/// GPU and temperature counters, kept open by the watchdog (GPU is a rate: needs two samples).
struct Pdh {
    q: isize,
    gpu: isize,
    temp: isize,
}

impl Pdh {
    fn open() -> Option<Self> {
        unsafe {
            let mut q = 0isize;
            if PdhOpenQueryW(std::ptr::null(), 0, &mut q) != 0 {
                return None;
            }
            let (mut gpu, mut temp) = (0isize, 0isize);
            let g = wide(r"\GPU Engine(*engtype_3D)\Utilization Percentage");
            let t = wide(r"\Thermal Zone Information(*)\Temperature");
            if PdhAddEnglishCounterW(q, g.as_ptr(), 0, &mut gpu) != 0 {
                gpu = 0;
            }
            if PdhAddEnglishCounterW(q, t.as_ptr(), 0, &mut temp) != 0 {
                temp = 0;
            }
            let _ = PdhCollectQueryData(q);
            Some(Pdh { q, gpu, temp })
        }
    }

    fn items(counter: isize) -> Vec<(String, f64)> {
        if counter == 0 {
            return Vec::new();
        }
        unsafe {
            let (mut size, mut count) = (0u32, 0u32);
            let r = PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, std::ptr::null_mut());
            if r != PDH_MORE_DATA || size == 0 {
                return Vec::new();
            }
            let mut buf = vec![0u8; size as usize + 16];
            let items = buf.as_mut_ptr() as *mut PdhItem;
            if PdhGetFormattedCounterArrayW(counter, PDH_FMT_DOUBLE, &mut size, &mut count, items) != 0 {
                return Vec::new();
            }
            (0..count as usize)
                .filter_map(|i| {
                    let it = &*items.add(i);
                    if it.status > 1 || it.name.is_null() {
                        return None;
                    }
                    let mut n = 0;
                    while *it.name.add(n) != 0 && n < 512 {
                        n += 1;
                    }
                    let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.name, n));
                    Some((name, it.value))
                })
                .collect()
        }
    }

    /// (GPU 3D %, temperatures °C) since the previous call.
    fn sample(&self) -> (Option<f64>, Vec<(String, f64)>) {
        if unsafe { PdhCollectQueryData(self.q) } != 0 {
            return (None, Vec::new());
        }
        let g = Self::items(self.gpu);
        let gpu = (!g.is_empty()).then(|| g.iter().map(|(_, v)| v).sum::<f64>().clamp(0.0, 100.0));
        let temps = Self::items(self.temp)
            .into_iter()
            .map(|(z, k)| (zone_name(&z), ((k - 273.15) * 10.0).round() / 10.0))
            .filter(|(_, c)| *c > 0.0 && *c < 150.0)
            .collect();
        (gpu, temps)
    }
}

/// "\_TZ.CPUZ" → "cpu", "\_SB.PCI0.LPCB.EC0.HEPZ" → "hepz" (pure). ACPI zone names are the
/// firmware's own; the common ones get a plain word.
pub fn zone_name(raw: &str) -> String {
    let last = raw.rsplit(['.', '\\']).next().unwrap_or(raw).to_lowercase();
    match last.as_str() {
        "cpuz" | "cpu" => "cpu",
        "gfxz" | "gpuz" => "gpu",
        "batz" => "battery",
        "chgz" => "charger",
        "pchz" => "chipset",
        "extz" => "case",
        "locz" | "skin" => "surface",
        other => other,
    }
    .to_string()
}

// SAFETY: the PDH query handle is only used from the watchdog thread after being moved there.
unsafe impl Send for Pdh {}

// ---------------------------------------------------------------------------------------------
// Warning levels (pure)

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub opened_warn: usize,
    pub opened_crit: usize,
    pub growth_warn: i64,
    pub growth_crit: i64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            opened_warn: 15,
            opened_crit: 30,
            growth_warn: 40,
            growth_crit: 100,
        }
    }
}

fn limits() -> Limits {
    // Test-only overrides (interactive tests use small numbers).
    let env = |k: &str, d: i64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let l = Limits::default();
    Limits {
        opened_warn: env("XTRAOS_HEALTH_OPENED_WARN", l.opened_warn as i64) as usize,
        opened_crit: env("XTRAOS_HEALTH_OPENED_CRIT", l.opened_crit as i64) as usize,
        growth_warn: env("XTRAOS_HEALTH_GROWTH_WARN", l.growth_warn),
        growth_crit: env("XTRAOS_HEALTH_GROWTH_CRIT", l.growth_crit),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    pub key: String,
    pub critical: bool,
    pub text: String,
}

/// The warnings for one reading (ADR 0050 table). `cpu_busy_s`: seconds CPU has been ≥ 95 %;
/// `opened`: windows/tabs the agent opened in the last 10 minutes; `growth`: per-app process
/// count increase since the session started (largest first).
pub fn assess(r: &Reading, cpu_busy_s: u64, opened: usize, growth: &[(String, i64)], l: &Limits) -> Vec<Warning> {
    let mut w = Vec::new();
    let mut add = |key: &str, critical: bool, text: String| {
        w.push(Warning { key: key.into(), critical, text });
    };
    if r.ram_total_gb > 0.0 && r.ram_pct >= 90.0 {
        add("ram", r.ram_pct >= 95.0, format!("RAM {:.0} % in use ({:.1} of {:.1} GB)", r.ram_pct, r.ram_used_gb, r.ram_total_gb));
    }
    if cpu_busy_s >= 30 {
        add("cpu", false, format!("CPU above 95 % for {cpu_busy_s} s"));
    }
    if let Some((zone, c)) = r.temps.iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
        if *c >= 90.0 {
            add("temp", *c >= 98.0, format!("{zone} is at {c:.0} °C"));
        }
    }
    let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into()).to_uppercase();
    for (d, free, total) in &r.disks {
        if d.eq_ignore_ascii_case(&sys) && (*free < 5.0 || *free < total * 0.05) {
            add("disk", *free < 1.0, format!("{d} has only {free:.1} GB free"));
        }
    }
    if opened >= l.opened_warn {
        add("opened", opened >= l.opened_crit, format!("the agent opened {opened} windows/tabs in 10 minutes"));
    }
    if let Some((app, n)) = growth.first() {
        if *n >= l.growth_warn {
            add("growth", *n >= l.growth_crit, format!("{app} has {n} more processes than when the session started (many tabs or windows?)"));
        }
    }
    w
}

// ---------------------------------------------------------------------------------------------
// Watchdog state

#[derive(Default)]
struct State {
    last: Option<Reading>,
    cpu_busy_since: Option<Instant>,
    baseline: HashMap<String, u32>,
    opened: VecDeque<Instant>,
    shown: HashMap<String, Instant>,
    approved_until: Option<Instant>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
static WATCHDOG: Once = Once::new();

fn with_state<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut g = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(State::default))
}

/// Starts the 5-second watchdog once (called on the first action and by `system health`).
pub fn ensure_watchdog() {
    WATCHDOG.call_once(|| {
        let _ = std::thread::Builder::new().name("xtraos-health".into()).spawn(|| {
            let pdh = Pdh::open();
            let mut prev_cpu = system_times();
            sleep(Duration::from_millis(500));
            loop {
                let now_cpu = system_times();
                let cpu = match (prev_cpu, now_cpu) {
                    (Some(a), Some(b)) => cpu_between(a, b),
                    _ => None,
                };
                prev_cpu = now_cpu;
                let (gpu, temps) = pdh.as_ref().map(Pdh::sample).unwrap_or((None, Vec::new()));
                let (used, total, pct) = ram();
                let r = Reading {
                    cpu_pct: cpu.map(|c| (c * 10.0).round() / 10.0),
                    ram_used_gb: (used * 10.0).round() / 10.0,
                    ram_total_gb: (total * 10.0).round() / 10.0,
                    ram_pct: (pct * 10.0).round() / 10.0,
                    gpu_pct: gpu.map(|g| (g * 10.0).round() / 10.0),
                    temps,
                    disks: fixed_disks(),
                    procs: counts(&processes()),
                };
                let warnings = with_state(|s| {
                    if r.cpu_pct.unwrap_or(0.0) >= 95.0 {
                        s.cpu_busy_since.get_or_insert_with(Instant::now);
                    } else {
                        s.cpu_busy_since = None;
                    }
                    for (k, &n) in &r.procs {
                        let b = s.baseline.entry(k.clone()).or_insert(n);
                        if n < *b {
                            *b = n; // the baseline follows apps closing, never rises
                        }
                    }
                    s.last = Some(r);
                    current(s)
                });
                // The user sees each warning on the banner at most once a minute.
                for w in &warnings {
                    let show = with_state(|s| {
                        let due = s.shown.get(&w.key).is_none_or(|t| t.elapsed() >= Duration::from_secs(60));
                        if due {
                            s.shown.insert(w.key.clone(), Instant::now());
                        }
                        due
                    });
                    if show {
                        crate::safety::activity(&format!("⚠ {}", w.text));
                        crate::safety::activity_end();
                    }
                }
                sleep(Duration::from_secs(5));
            }
        });
    });
}

fn opened_recent(s: &mut State) -> usize {
    while s.opened.front().is_some_and(|t| t.elapsed() > Duration::from_secs(600)) {
        s.opened.pop_front();
    }
    s.opened.len()
}

fn growth(s: &State) -> Vec<(String, i64)> {
    let Some(r) = &s.last else { return Vec::new() };
    let mut g: Vec<(String, i64)> = r
        .procs
        .iter()
        .map(|(k, &n)| (k.trim_end_matches(".exe").to_string(), n as i64 - *s.baseline.get(k).unwrap_or(&n) as i64))
        .filter(|(_, d)| *d > 0)
        .collect();
    g.sort_by(|a, b| b.1.cmp(&a.1));
    g
}

fn current(s: &mut State) -> Vec<Warning> {
    let busy = s.cpu_busy_since.map_or(0, |t| t.elapsed().as_secs());
    let opened = opened_recent(s);
    let g = growth(s);
    match &s.last {
        Some(r) => assess(r, busy, opened, &g, &limits()),
        None => assess(&Reading::default(), 0, opened, &g, &limits()),
    }
}

/// The agent opened a window or tab (launch_app, browser open_tab).
pub fn note_opened() {
    with_state(|s| s.opened.push_back(Instant::now()));
}

/// One line for tool results while something is wrong (None when all is well).
pub fn warning_text() -> Option<String> {
    let w = with_state(current);
    if w.is_empty() {
        return None;
    }
    let mut t: Vec<String> = w.iter().map(|x| x.text.clone()).collect();
    t.dedup();
    Some(format!(
        "{} — check whether you are opening too much; cleanup closes the windows you opened",
        t.join("; ")
    ))
}

/// `status.health`: the last reading's headline numbers and the warnings.
pub fn status_json() -> Value {
    ensure_watchdog();
    with_state(|s| {
        let w = current(s);
        json!({
            "cpu_pct": s.last.as_ref().and_then(|r| r.cpu_pct),
            "ram_pct": s.last.as_ref().map(|r| r.ram_pct),
            "warnings": w.iter().map(|x| x.text.clone()).collect::<Vec<_>>(),
        })
    })
}

/// `system {op: "health"}`: every reading plus the top memory users.
pub fn report() -> Result<Value, ActError> {
    ensure_watchdog();
    let t0 = Instant::now();
    while with_state(|s| s.last.is_none()) && t0.elapsed() < Duration::from_secs(3) {
        sleep(Duration::from_millis(100));
    }
    let wifi = wifi_signal();
    let top = top_apps(5);
    let battery = crate::sysctl::battery_json().ok();
    Ok(with_state(|s| {
        let w = current(s);
        let r = s.last.clone().unwrap_or_default();
        let top_line: Vec<String> = top.iter().map(|(a, n, mb)| format!("{a} ×{n} {:.1} GB", *mb as f64 / 1024.0)).collect();
        let temps_line = if r.temps.is_empty() {
            "temperatures: not available on this PC".to_string()
        } else {
            format!(
                "temps: {}",
                r.temps.iter().map(|(z, c)| format!("{z} {c:.0} °C")).collect::<Vec<_>>().join(", ")
            )
        };
        let text = format!(
            "CPU {} · RAM {:.1}/{:.1} GB ({:.0} %) · GPU {} · Wi-Fi {}{}\n{temps_line}\ndisks: {}\ntop memory: {}{}",
            r.cpu_pct.map_or("?".into(), |c| format!("{c:.0} %")),
            r.ram_used_gb,
            r.ram_total_gb,
            r.ram_pct,
            r.gpu_pct.map_or("n/a".into(), |g| format!("{g:.0} %")),
            wifi.map_or("not connected".into(), |q| format!("{q} %")),
            battery
                .as_ref()
                .filter(|b| b["battery"] == true)
                .map(|b| format!(" · battery {} %{}", b["percent"], if b["plugged_in"] == true { " (plugged in)" } else { "" }))
                .unwrap_or_default(),
            r.disks.iter().map(|(d, f, t)| format!("{d} {f:.0} of {t:.0} GB free")).collect::<Vec<_>>().join(", "),
            top_line.join(", "),
            if w.is_empty() { String::new() } else { format!("\n⚠ {}", w.iter().map(|x| x.text.clone()).collect::<Vec<_>>().join("; ")) },
        );
        json!({
            "text": text,
            "cpu_pct": r.cpu_pct,
            "ram": {"used_gb": r.ram_used_gb, "total_gb": r.ram_total_gb, "pct": r.ram_pct},
            "gpu_pct": r.gpu_pct,
            "temps": r.temps.iter().map(|(z, c)| json!({"zone": z, "c": c})).collect::<Vec<_>>(),
            "disks": r.disks.iter().map(|(d, f, t)| json!({"drive": d, "free_gb": (f * 10.0).round() / 10.0, "pct_free": (f * 1000.0 / t).round() / 10.0})).collect::<Vec<_>>(),
            "wifi": wifi.map(|q| json!({"signal_pct": q})),
            "battery": battery,
            "top": top.iter().map(|(a, n, mb)| json!({"app": a, "processes": n, "mb": mb})).collect::<Vec<_>>(),
            "warnings": w.iter().map(|x| json!({"text": x.text, "critical": x.critical})).collect::<Vec<_>>(),
        })
    }))
}

// ---------------------------------------------------------------------------------------------
// Brake (ADR 0050 §4)

/// Does this action open more windows or tabs? (pure)
pub fn opens_more(method: &str, p: &Value) -> bool {
    match method {
        "launch_app" => true,
        "browser" => p.get("action").and_then(Value::as_str) == Some("open_tab"),
        "run_skill" => p.get("name").and_then(Value::as_str) == Some("files.open"),
        _ => false,
    }
}

/// At a critical level, an action that opens more asks the user first. A yes holds for 2 minutes.
pub fn brake(method: &str, p: &Value) -> Result<(), ActError> {
    ensure_watchdog();
    if !opens_more(method, p) {
        return Ok(());
    }
    let (critical, approved) = with_state(|s| {
        let w = current(s);
        let crit: Vec<String> = w.iter().filter(|x| x.critical).map(|x| x.text.clone()).collect();
        (crit, s.approved_until.is_some_and(|t| Instant::now() < t))
    });
    if critical.is_empty() || approved {
        note_opened();
        return Ok(());
    }
    let msg = format!(
        "The PC is under strain:\n\n• {}\n\nThe AI agent wants to open more ({method}). Allow it?\n\nNo stops this step; the agent can still read and close windows.",
        critical.join("\n• ")
    );
    crate::safety::activity("waiting for you: the PC is under strain");
    let answer = crate::safety::prompt(
        "XtraOS Agent — the PC is under strain",
        &msg,
        vec!["Allow".into(), "Stop it".into()],
        1,
        // No answer = "Stop it". Tests shorten the wait; nothing can answer for the user.
        Duration::from_millis(
            std::env::var("XTRAOS_HEALTH_PROMPT_MS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(120_000),
        ),
    )?;
    if answer == Some(0) {
        with_state(|s| s.approved_until = Some(Instant::now() + Duration::from_secs(120)));
        note_opened();
        return Ok(());
    }
    Err(ActError {
        code: "BLOCKED_BY_POLICY",
        message: format!("the PC is under strain ({}); the user did not allow opening more", critical.join("; ")),
        hint: "Do not open more. Close what you opened (cleanup) and tell the user.",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reading() -> Reading {
        Reading {
            cpu_pct: Some(10.0),
            ram_used_gb: 8.0,
            ram_total_gb: 16.0,
            ram_pct: 50.0,
            gpu_pct: Some(5.0),
            temps: vec![("cpuz".into(), 70.0)],
            disks: vec![("C:".into(), 60.0, 500.0)],
            procs: HashMap::new(),
        }
    }

    #[test]
    fn a_healthy_pc_has_no_warnings() {
        assert!(assess(&reading(), 0, 3, &[], &Limits::default()).is_empty());
    }

    #[test]
    fn levels_follow_the_adr_table() {
        let l = Limits::default();
        let mut r = reading();
        r.ram_pct = 92.0;
        let w = assess(&r, 0, 0, &[], &l);
        assert_eq!((w[0].key.as_str(), w[0].critical), ("ram", false));
        r.ram_pct = 96.0;
        assert!(assess(&r, 0, 0, &[], &l)[0].critical);
        let mut r = reading();
        r.temps = vec![("cpuz".into(), 99.0), ("batz".into(), 40.0)];
        let w = assess(&r, 0, 0, &[], &l);
        assert!(w[0].text.contains("cpuz") && w[0].critical, "{w:?}");
        let mut r = reading();
        r.disks = vec![("C:".into(), 0.5, 500.0)];
        assert!(assess(&r, 0, 0, &[], &l)[0].critical);
        assert_eq!(assess(&reading(), 31, 0, &[], &l)[0].key, "cpu");
        let w = assess(&reading(), 0, 16, &[], &l);
        assert_eq!((w[0].key.as_str(), w[0].critical), ("opened", false));
        assert!(assess(&reading(), 0, 30, &[], &l)[0].critical);
        let g = vec![("chrome".to_string(), 120)];
        let w = assess(&reading(), 0, 0, &g, &l);
        assert!(w[0].text.contains("chrome") && w[0].critical, "{w:?}");
        assert!(assess(&reading(), 0, 0, &[("chrome".into(), 10)], &l).is_empty());
    }

    #[test]
    fn zone_names_are_plain() {
        assert_eq!(zone_name(r"\_TZ.CPUZ"), "cpu");
        assert_eq!(zone_name("TZ.BATZ"), "battery");
        assert_eq!(zone_name(r"\_SB.PCI0.LPCB.EC0.HEPZ"), "hepz");
    }

    #[test]
    fn cpu_percent_from_two_samples() {
        assert_eq!(cpu_between((100, 1000), (150, 1100)), Some(50.0));
        assert_eq!(cpu_between((100, 1000), (200, 1100)), Some(0.0));
        assert_eq!(cpu_between((100, 1000), (100, 1000)), None);
    }

    #[test]
    fn only_opening_actions_are_braked() {
        assert!(opens_more("launch_app", &json!({"app": "notepad"})));
        assert!(opens_more("browser", &json!({"action": "open_tab"})));
        assert!(!opens_more("browser", &json!({"action": "list_tabs"})));
        assert!(opens_more("run_skill", &json!({"name": "files.open"})));
        assert!(!opens_more("close_window", &json!({})));
        assert!(!opens_more("observe", &json!({})));
    }

    #[test]
    fn live_readings_work_without_admin() {
        let (used, total, pct) = ram();
        assert!(total > 0.5 && used > 0.0 && pct > 0.0 && pct <= 100.0);
        assert!(!fixed_disks().is_empty());
        assert!(processes().len() > 10);
        let a = system_times().unwrap();
        sleep(Duration::from_millis(200));
        assert!(cpu_between(a, system_times().unwrap()).is_some());
    }
}
