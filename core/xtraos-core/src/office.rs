//! Office pack, slice 1 (P7-M1, ADR 0055): Excel read/write and PDF export through Office
//! automation (COM late binding, `IDispatch`).
//!
//! Every call runs in **its own hidden Office instance** on an STA thread, never in the user's
//! open Excel/Word: macros forced off before anything is opened, alerts off, links not updated,
//! an empty password so a protected file fails instead of showing a password dialog. The
//! instance is quit and released at the end; a hang past the time limit ends only the Office
//! process this call started.
#![allow(clippy::upper_case_acronyms)]

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{json, Value};
use windows::core::{BSTR, GUID, HSTRING, PCWSTR};
use windows::Win32::Foundation::VARIANT_BOOL;
use windows::Win32::System::Com::{
    CLSIDFromProgID, CoCreateInstance, CoInitializeEx, CoUninitialize, IDispatch, CLSCTX_LOCAL_SERVER,
    COINIT_APARTMENTTHREADED, DISPATCH_FLAGS, DISPATCH_METHOD, DISPATCH_PROPERTYGET,
    DISPATCH_PROPERTYPUT, DISPPARAMS,
};
use windows::Win32::System::Ole::DISPID_PROPERTYPUT;
use windows::Win32::System::Variant::{
    VariantClear, VARIANT, VT_BOOL, VT_BSTR, VT_DATE, VT_DISPATCH, VT_EMPTY, VT_ERROR, VT_I2, VT_I4,
    VT_R4, VT_R8,
};

use crate::input::ActError;

const LIMIT: Duration = Duration::from_secs(120);
const READ_CELLS: usize = 2_000;
const WRITE_CELLS: usize = 1_000;

fn bad(m: impl Into<String>) -> ActError {
    ActError { code: "INVALID_ARGS", message: m.into(), hint: "See run_skill office.* in docs/04-mcp-tool-contract.md." }
}

fn human(m: impl Into<String>, hint: &'static str) -> ActError {
    ActError { code: "NEEDS_HUMAN", message: m.into(), hint }
}

fn com(what: &str, e: windows::core::Error) -> ActError {
    ActError {
        code: "INTERNAL",
        message: format!("{what}: {}", e.message()),
        hint: "The file may be damaged, password-protected or open elsewhere; check it in Office.",
    }
}

// ---------------------------------------------------------------------------------------------
// VARIANT helpers

/// A VARIANT that frees itself.
struct Var(VARIANT);

impl Drop for Var {
    fn drop(&mut self) {
        unsafe {
            let _ = VariantClear(&mut self.0);
        }
    }
}

fn var_i32(n: i32) -> Var {
    let mut v = VARIANT::default();
    unsafe {
        (*v.Anonymous.Anonymous).vt = VT_I4;
        (*v.Anonymous.Anonymous).Anonymous.lVal = n;
    }
    Var(v)
}

fn var_f64(n: f64) -> Var {
    let mut v = VARIANT::default();
    unsafe {
        (*v.Anonymous.Anonymous).vt = VT_R8;
        (*v.Anonymous.Anonymous).Anonymous.dblVal = n;
    }
    Var(v)
}

fn var_bool(b: bool) -> Var {
    let mut v = VARIANT::default();
    unsafe {
        (*v.Anonymous.Anonymous).vt = VT_BOOL;
        (*v.Anonymous.Anonymous).Anonymous.boolVal = VARIANT_BOOL(if b { -1 } else { 0 });
    }
    Var(v)
}

fn var_str(s: &str) -> Var {
    let mut v = VARIANT::default();
    unsafe {
        (*v.Anonymous.Anonymous).vt = VT_BSTR;
        (*v.Anonymous.Anonymous).Anonymous.bstrVal = std::mem::ManuallyDrop::new(BSTR::from(s));
    }
    Var(v)
}

/// An optional argument left out: VT_ERROR with DISP_E_PARAMNOTFOUND (not VT_EMPTY, which
/// Office reads as a real but invalid value).
fn var_missing() -> Var {
    let mut v = VARIANT::default();
    unsafe {
        (*v.Anonymous.Anonymous).vt = VT_ERROR;
        (*v.Anonymous.Anonymous).Anonymous.scode = 0x8002_0004u32 as i32;
    }
    Var(v)
}

/// A JSON cell value → VARIANT (numbers, text, booleans; null = empty).
fn var_json(v: &Value) -> Var {
    match v {
        Value::Number(n) => var_f64(n.as_f64().unwrap_or(0.0)),
        Value::Bool(b) => var_bool(*b),
        Value::String(s) => var_str(s),
        _ => Var(VARIANT::default()),
    }
}

/// A cell's VARIANT → JSON (pure over the decoded parts).
fn var_to_json(v: &VARIANT) -> Value {
    unsafe {
        let inner = &v.Anonymous.Anonymous;
        match inner.vt {
            VT_EMPTY => Value::Null,
            VT_BSTR => json!(inner.Anonymous.bstrVal.to_string()),
            VT_R8 | VT_DATE => num(inner.Anonymous.dblVal),
            VT_R4 => num(inner.Anonymous.fltVal as f64),
            VT_I4 => json!(inner.Anonymous.lVal),
            VT_I2 => json!(inner.Anonymous.iVal),
            VT_BOOL => json!(inner.Anonymous.boolVal.0 != 0),
            VT_ERROR => json!("#ERR"),
            _ => json!("?"),
        }
    }
}

/// Whole numbers without ".0" (pure).
pub fn num(f: f64) -> Value {
    if f.fract() == 0.0 && f.abs() < 9.0e15 {
        json!(f as i64)
    } else {
        json!(f)
    }
}

fn var_dispatch(v: &Var) -> Option<Disp> {
    unsafe {
        let inner = &v.0.Anonymous.Anonymous;
        if inner.vt == VT_DISPATCH {
            (*inner.Anonymous.pdispVal).clone().map(Disp)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------------------------
// IDispatch late binding

#[derive(Clone)]
struct Disp(IDispatch);

impl Disp {
    fn id(&self, name: &str) -> Result<i32, ActError> {
        let w = HSTRING::from(name);
        let names = [PCWSTR(w.as_ptr())];
        let mut id = 0i32;
        unsafe { self.0.GetIDsOfNames(&GUID::zeroed(), names.as_ptr(), 1, 0x0409, &mut id) }
            .map_err(|e| com(&format!("unknown member {name}"), e))?;
        Ok(id)
    }

    fn invoke(&self, name: &str, flags: DISPATCH_FLAGS, args: &mut [VARIANT], put: bool) -> Result<Var, ActError> {
        let id = self.id(name)?;
        // DISPPARAMS take the arguments in reverse order.
        args.reverse();
        let mut named = DISPID_PROPERTYPUT;
        let params = DISPPARAMS {
            rgvarg: if args.is_empty() { std::ptr::null_mut() } else { args.as_mut_ptr() },
            rgdispidNamedArgs: if put { &mut named } else { std::ptr::null_mut() },
            cArgs: args.len() as u32,
            cNamedArgs: u32::from(put),
        };
        let mut out = VARIANT::default();
        let r = unsafe { self.0.Invoke(id, &GUID::zeroed(), 0x0409, flags, &params, Some(&mut out), None, None) };
        args.reverse();
        r.map_err(|e| com(name, e))?;
        Ok(Var(out))
    }

    fn get(&self, name: &str, args: Vec<Var>) -> Result<Var, ActError> {
        let mut raw: Vec<VARIANT> = args.iter().map(|a| unsafe { std::ptr::read(&a.0) }).collect();
        let r = self.invoke(name, DISPATCH_FLAGS(DISPATCH_METHOD.0 | DISPATCH_PROPERTYGET.0), &mut raw, false);
        std::mem::forget(raw); // `args` still owns (and frees) the values
        r
    }

    fn obj(&self, name: &str, args: Vec<Var>) -> Result<Disp, ActError> {
        let v = self.get(name, args)?;
        var_dispatch(&v).ok_or_else(|| bad(format!("{name} is not an object")))
    }

    fn put(&self, name: &str, value: Var) -> Result<(), ActError> {
        let mut raw = [unsafe { std::ptr::read(&value.0) }];
        let r = self.invoke(name, DISPATCH_PROPERTYPUT, &mut raw, true);
        std::mem::forget(raw);
        drop(value);
        r.map(|_| ())
    }
}

// ---------------------------------------------------------------------------------------------
// One private, hidden Office instance

const PROC: &[(&str, &str)] = &[
    ("Excel.Application", "excel.exe"),
    ("Word.Application", "winword.exe"),
    ("PowerPoint.Application", "powerpnt.exe"),
];

fn exe_of(progid: &str) -> &'static str {
    PROC.iter().find(|(p, _)| *p == progid).map(|(_, e)| *e).unwrap_or("")
}

struct App {
    app: Disp,
}

thread_local! {
    /// Where App::start reports the process id of the instance it created (set by `run`).
    static OUR_PID: std::cell::RefCell<Option<std::sync::Arc<std::sync::Mutex<Option<u32>>>>> =
        const { std::cell::RefCell::new(None) };
}

fn pids_of(exe: &str) -> Vec<u32> {
    crate::health::processes().into_iter().filter(|(_, n)| n == exe).map(|(p, _)| p).collect()
}

impl App {
    fn start(progid: &'static str) -> Result<App, ActError> {
        let clsid = unsafe { CLSIDFromProgID(&HSTRING::from(progid)) }
            .map_err(|_| human(format!("{progid} is not installed"), "Install Microsoft Office desktop, or do it by hand."))?;
        let exe = exe_of(progid);
        let before = pids_of(exe);
        let d: IDispatch = unsafe { CoCreateInstance(&clsid, None, CLSCTX_LOCAL_SERVER) }.map_err(|e| com(&format!("starting {progid}"), e))?;
        // The one new process is this call's private instance (several new ones: unknown, and
        // then nothing is ever ended by force).
        let new: Vec<u32> = pids_of(exe).into_iter().filter(|p| !before.contains(p)).collect();
        if new.len() == 1 {
            OUR_PID.with(|c| {
                if let Some(slot) = c.borrow().as_ref() {
                    *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(new[0]);
                }
            });
        }
        let app = Disp(d);
        // Macros off before anything is opened (msoAutomationSecurityForceDisable = 3).
        app.put("AutomationSecurity", var_i32(3))?;
        match progid {
            "Excel.Application" => {
                let _ = app.put("Visible", var_bool(false));
                app.put("DisplayAlerts", var_bool(false))?;
                // Files the user double-clicks must not open in this hidden instance.
                let _ = app.put("IgnoreRemoteRequests", var_bool(true));
            }
            "Word.Application" => {
                let _ = app.put("Visible", var_bool(false));
                app.put("DisplayAlerts", var_i32(0))?; // wdAlertsNone
            }
            _ => {} // PowerPoint: an invisible app window is the default under automation
        }
        Ok(App { app })
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.app.get("Quit", Vec::new());
    }
}

/// Runs `f` on its own STA thread with the time limit. Afterwards the private instance must be
/// gone: Office can linger after Quit (seen: one hidden Excel stayed), so it gets 10 s, then
/// that one process — and only it — is ended. On a hang it is ended at once.
fn run<T: Send + 'static>(progid: &'static str, f: impl FnOnce() -> Result<T, ActError> + Send + 'static) -> Result<T, ActError> {
    let exe = exe_of(progid);
    let ours = std::sync::Arc::new(std::sync::Mutex::new(None::<u32>));
    let slot = ours.clone();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        OUR_PID.with(|c| *c.borrow_mut() = Some(slot));
        let init = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        let r = f(); // every COM reference of this call is released when f returns
        if init.is_ok() {
            unsafe { CoUninitialize() };
        }
        let _ = tx.send(r);
    });
    let pid = || *ours.lock().unwrap_or_else(|e| e.into_inner());
    match rx.recv_timeout(LIMIT) {
        Ok(r) => {
            // The result goes back at once; the private instance gets 10 s to exit in the
            // background (Office often takes 5–10 s on a busy PC), then it is ended.
            if let Some(p) = pid() {
                std::thread::spawn(move || {
                    let t0 = std::time::Instant::now();
                    while pids_of(exe).contains(&p) && t0.elapsed() < Duration::from_secs(10) {
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    if pids_of(exe).contains(&p) {
                        end_process(p);
                    }
                });
            }
            r
        }
        Err(_) => {
            if let Some(p) = pid() {
                end_process(p);
            }
            Err(ActError {
                code: "TIMEOUT",
                message: format!("Office did not finish within {} s; its hidden instance was ended", LIMIT.as_secs()),
                hint: "The file may show a dialog (repair, password); open it in Office by hand.",
            })
        }
    }
}

fn end_process(pid: u32) {
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut std::ffi::c_void;
        fn TerminateProcess(h: *mut std::ffi::c_void, code: u32) -> i32;
        fn CloseHandle(h: *mut std::ffi::c_void) -> i32;
    }
    unsafe {
        let h = OpenProcess(0x0001, 0, pid); // PROCESS_TERMINATE
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Skills

fn str_arg<'a>(a: &'a Value, k: &str) -> Result<&'a str, ActError> {
    a.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| bad(format!("'{k}' is required")))
}

/// Which Office app opens a file for PDF export (pure).
pub fn app_for(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_string_lossy().to_lowercase();
    Some(match ext.as_str() {
        "docx" | "doc" | "rtf" | "odt" | "docm" => "Word.Application",
        "xlsx" | "xls" | "xlsm" | "csv" | "ods" | "xlsb" => "Excel.Application",
        "pptx" | "ppt" | "odp" | "pptm" => "PowerPoint.Application",
        _ => return None,
    })
}

/// The PDF next to a document (pure): report.docx → report.pdf.
pub fn pdf_target(path: &Path, out: Option<&str>) -> PathBuf {
    match out {
        Some(o) => PathBuf::from(o),
        None => path.with_extension("pdf"),
    }
}

fn open_workbook(app: &Disp, path: &Path, read_only: bool) -> Result<Disp, ActError> {
    let books = app.obj("Workbooks", Vec::new())?;
    // Open(Filename, UpdateLinks=0, ReadOnly, Format, Password="") — an empty password makes a
    // protected file fail instead of asking.
    books
        .obj(
            "Open",
            vec![var_str(&path.to_string_lossy()), var_i32(0), var_bool(read_only), var_missing(), var_str("")],
        )
        .map_err(|e| human(format!("could not open {} (password-protected, damaged or not a workbook): {}", path.display(), e.message), "Open it in Excel by hand."))
}

fn sheet_of(wb: &Disp, name: Option<&str>) -> Result<Disp, ActError> {
    let sheets = wb.obj("Worksheets", Vec::new())?;
    match name {
        Some(n) => sheets.obj("Item", vec![var_str(n)]).map_err(|_| bad(format!("no sheet named \"{n}\""))),
        None => sheets.obj("Item", vec![var_i32(1)]),
    }
}

fn count(d: &Disp, name: &str) -> Result<i32, ActError> {
    let v = d.obj(name, Vec::new())?.get("Count", Vec::new())?;
    Ok(var_to_json(&v.0).as_i64().unwrap_or(0) as i32)
}

pub fn excel_read(a: &Value) -> Result<Value, ActError> {
    let path = crate::skills::checked(str_arg(a, "path")?)?;
    if !path.is_file() {
        return Err(bad(format!("{} does not exist", path.display())));
    }
    let range = str_arg(a, "range")?.to_string();
    let sheet = a.get("sheet").and_then(Value::as_str).map(String::from);
    let shown = path.display().to_string();
    run("Excel.Application", move || {
        let x = App::start("Excel.Application")?;
        let wb = open_workbook(&x.app, &path, true)?;
        let r = (|| {
            let ws = sheet_of(&wb, sheet.as_deref())?;
            let rg = ws.obj("Range", vec![var_str(&range)]).map_err(|_| bad(format!("'{range}' is not a range like A1:C10")))?;
            let (rows, cols) = (count(&rg, "Rows")?, count(&rg, "Columns")?);
            if (rows * cols) as usize > READ_CELLS {
                return Err(bad(format!("{range} has {} cells; read at most {READ_CELLS} at a time", rows * cols)));
            }
            let mut out = Vec::new();
            for r in 1..=rows {
                let mut row = Vec::new();
                for c in 1..=cols {
                    let cell = rg.obj("Cells", vec![var_i32(r), var_i32(c)])?;
                    row.push(var_to_json(&cell.get("Value2", Vec::new())?.0));
                }
                out.push(row);
            }
            let name = var_to_json(&ws.get("Name", Vec::new())?.0);
            Ok((out, name))
        })();
        let _ = wb.get("Close", vec![var_bool(false)]);
        let (values, sheet_name) = r?;
        drop(x);
        let text: Vec<String> = values
            .iter()
            .map(|row| row.iter().map(|v| match v { Value::Null => String::new(), Value::String(s) => s.clone(), o => o.to_string() }).collect::<Vec<_>>().join("\t"))
            .collect();
        Ok(json!({
            "text": format!("{shown} [{}] {range}\n{}", sheet_name.as_str().unwrap_or("?"), text.join("\n")),
            "values": values,
            "sheet": sheet_name,
        }))
    })
}

/// The `values` argument: rows of cells, at most WRITE_CELLS (pure).
pub fn write_values(a: &Value) -> Result<Vec<Vec<Value>>, ActError> {
    let rows = a.get("values").and_then(Value::as_array).filter(|r| !r.is_empty()).ok_or_else(|| bad("'values' is required: rows of cells, e.g. [[\"Item\", \"Qty\"], [\"Pens\", 12]]"))?;
    let mut out = Vec::new();
    let mut n = 0;
    for r in rows {
        let cells = r.as_array().ok_or_else(|| bad("each row of 'values' is a list of cells"))?;
        for c in cells {
            if c.is_object() || c.is_array() {
                return Err(bad("a cell is a number, text, true/false or null"));
            }
        }
        n += cells.len();
        out.push(cells.clone());
    }
    if n > WRITE_CELLS {
        return Err(bad(format!("{n} cells; write at most {WRITE_CELLS} at a time")));
    }
    Ok(out)
}

/// Widest column after autofit, in Excel's character units.
const MAX_COL_WIDTH: f64 = 60.0;

/// Formatting arguments of excel_write (pure): `autofit` (default true), `bold` ranges (≤ 20).
pub fn format_args(a: &Value) -> Result<(bool, Vec<String>), ActError> {
    let autofit = match a.get("autofit") {
        None | Some(Value::Null) => true,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad("'autofit' must be true or false")),
    };
    let bold: Vec<String> = match a.get("bold") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(v)) => v
            .iter()
            .map(|x| {
                x.as_str()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty() && s.len() <= 32 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == ':' || c == '$'))
                    .ok_or_else(|| bad("'bold' is a list of ranges like \"A1:D1\""))
            })
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(bad("'bold' is a list of ranges like \"A1:D1\"")),
    };
    if bold.len() > 20 {
        return Err(bad("at most 20 'bold' ranges"));
    }
    Ok((autofit, bold))
}

pub fn excel_write(a: &Value) -> Result<Value, ActError> {
    let path = crate::skills::checked(str_arg(a, "path")?)?;
    let save_as = match a.get("save_as").and_then(Value::as_str) {
        Some(s) => {
            let p = crate::skills::checked(s)?;
            if p.exists() {
                return Err(bad(format!("{} already exists (never overwrites)", p.display())));
            }
            Some(p)
        }
        None => None,
    };
    let new_book = !path.exists() && save_as.is_none();
    if !path.exists() && !new_book {
        return Err(bad(format!("{} does not exist", path.display())));
    }
    let range = str_arg(a, "range")?.to_string();
    let sheet = a.get("sheet").and_then(Value::as_str).map(String::from);
    let values = write_values(a)?;
    let (autofit, bold) = format_args(a)?;
    let cells: usize = values.iter().map(Vec::len).sum();
    let target = save_as.clone().unwrap_or_else(|| path.clone());
    run("Excel.Application", move || {
        let x = App::start("Excel.Application")?;
        let wb = if new_book {
            x.app.obj("Workbooks", Vec::new())?.obj("Add", Vec::new())?
        } else {
            let wb = open_workbook(&x.app, &path, false)?;
            if var_to_json(&wb.get("ReadOnly", Vec::new())?.0) == json!(true) && save_as.is_none() {
                let _ = wb.get("Close", vec![var_bool(false)]);
                return Err(human(format!("{} is open elsewhere (Excel opened it read-only)", path.display()), "Ask the user to close it, or pass save_as to write a copy."));
            }
            wb
        };
        let r = (|| {
            let ws = sheet_of(&wb, sheet.as_deref())?;
            let start = ws.obj("Range", vec![var_str(&range)]).map_err(|_| bad(format!("'{range}' is not a cell like B2")))?;
            for (i, row) in values.iter().enumerate() {
                for (j, v) in row.iter().enumerate() {
                    let cell = start.obj("Cells", vec![var_i32(i as i32 + 1), var_i32(j as i32 + 1)])?;
                    cell.put("Value2", var_json(v))?;
                }
            }
            // Readable without touching it (D1 demo: dates showed as ######, headers plain).
            for r in &bold {
                ws.obj("Range", vec![var_str(r)])
                    .map_err(|_| bad(format!("bold: '{r}' is not a range like A1:D1")))?
                    .obj("Font", Vec::new())?
                    .put("Bold", var_bool(true))?;
            }
            if autofit {
                let cols = ws.obj("UsedRange", Vec::new())?.obj("Columns", Vec::new())?;
                cols.get("AutoFit", Vec::new())?;
                let n = var_to_json(&cols.get("Count", Vec::new())?.0).as_i64().unwrap_or(0) as i32;
                for i in 1..=n.min(200) {
                    let c = cols.obj("Item", vec![var_i32(i)])?;
                    // Very long text (file names) would make a column wider than the screen.
                    if var_to_json(&c.get("ColumnWidth", Vec::new())?.0).as_f64().unwrap_or(0.0) > MAX_COL_WIDTH {
                        c.put("ColumnWidth", var_f64(MAX_COL_WIDTH))?;
                    }
                }
            }
            if new_book || save_as.is_some() {
                // xlOpenXMLWorkbook = 51 (.xlsx)
                wb.get("SaveAs", vec![var_str(&target.to_string_lossy()), var_i32(51)])?;
            } else {
                wb.get("Save", Vec::new())?;
            }
            Ok(())
        })();
        let _ = wb.get("Close", vec![var_bool(false)]);
        r?;
        drop(x);
        Ok(json!({"text": format!("ok wrote {cells} cell(s) at {range} in {}", target.display()), "path": target.display().to_string(), "cells": cells}))
    })
}

pub fn export_pdf(a: &Value) -> Result<Value, ActError> {
    let path = crate::skills::checked(str_arg(a, "path")?)?;
    if !path.is_file() {
        return Err(bad(format!("{} does not exist", path.display())));
    }
    let progid = app_for(&path).ok_or_else(|| bad("export_pdf takes a Word, Excel or PowerPoint file"))?;
    let out = pdf_target(&path, a.get("out").and_then(Value::as_str));
    let out = crate::skills::checked(&out.to_string_lossy())?;
    if out.exists() {
        return Err(bad(format!("{} already exists (never overwrites)", out.display())));
    }
    let (src, dst) = (path.clone(), out.clone());
    run(progid, move || {
        let x = App::start(progid)?;
        let s = src.to_string_lossy().to_string();
        let d = dst.to_string_lossy().to_string();
        match progid {
            "Excel.Application" => {
                let wb = open_workbook(&x.app, &src, true)?;
                let r = wb.get("ExportAsFixedFormat", vec![var_i32(0), var_str(&d)]); // xlTypePDF
                let _ = wb.get("Close", vec![var_bool(false)]);
                r?;
            }
            "Word.Application" => {
                let docs = x.app.obj("Documents", Vec::new())?;
                // Open(FileName, ConfirmConversions, ReadOnly, AddToRecentFiles, PasswordDocument)
                let doc = docs
                    .obj("Open", vec![var_str(&s), var_bool(false), var_bool(true), var_bool(false), var_str("")])
                    .map_err(|e| human(format!("could not open {} (password-protected or damaged): {}", src.display(), e.message), "Open it in Word by hand."))?;
                let r = doc.get("ExportAsFixedFormat", vec![var_str(&d), var_i32(17)]); // wdExportFormatPDF
                let _ = doc.get("Close", vec![var_i32(0)]); // wdDoNotSaveChanges
                r?;
            }
            _ => {
                let pres = x.app.obj("Presentations", Vec::new())?;
                // Open(FileName, ReadOnly, Untitled, WithWindow) — msoTrue = -1, msoFalse = 0
                let p = pres
                    .obj("Open", vec![var_str(&s), var_i32(-1), var_i32(0), var_i32(0)])
                    .map_err(|e| human(format!("could not open {} (password-protected or damaged): {}", src.display(), e.message), "Open it in PowerPoint by hand."))?;
                let r = p.get("SaveAs", vec![var_str(&d), var_i32(32)]); // ppSaveAsPDF
                let _ = p.get("Close", Vec::new());
                r?;
            }
        }
        drop(x);
        if !dst.exists() {
            return Err(com("export", windows::core::Error::from_hresult(windows::core::HRESULT(0x8000_4005u32 as i32))));
        }
        Ok(json!({"text": format!("ok saved {}", dst.display()), "path": dst.display().to_string()}))
    })
}

// ---------------------------------------------------------------------------------------------
// Word (slice 2, ADR 0056)

/// Word's text → plain lines (pure): paragraph `\r`, cell end `\x07`, line break `\x0b`.
pub fn word_text(raw: &str) -> String {
    raw.replace("\r\x07", "\t")
        .replace('\x07', "\t")
        .replace('\x0b', "\n")
        .replace('\r', "\n")
        .trim_end()
        .to_string()
}

fn open_doc(app: &Disp, path: &Path, read_only: bool) -> Result<Disp, ActError> {
    let docs = app.obj("Documents", Vec::new())?;
    // Open(FileName, ConfirmConversions, ReadOnly, AddToRecentFiles, PasswordDocument)
    docs.obj(
        "Open",
        vec![var_str(&path.to_string_lossy()), var_bool(false), var_bool(read_only), var_bool(false), var_str("")],
    )
    .map_err(|e| human(format!("could not open {} (password-protected or damaged): {}", path.display(), e.message), "Open it in Word by hand."))
}

pub fn word_read(a: &Value) -> Result<Value, ActError> {
    let path = crate::skills::checked(str_arg(a, "path")?)?;
    if !path.is_file() {
        return Err(bad(format!("{} does not exist", path.display())));
    }
    let max = a.get("max_chars").and_then(Value::as_u64).unwrap_or(20_000).clamp(100, 200_000) as usize;
    let shown = path.display().to_string();
    run("Word.Application", move || {
        let w = App::start("Word.Application")?;
        let doc = open_doc(&w.app, &path, true)?;
        let r = (|| {
            let raw = doc.obj("Content", Vec::new())?.get("Text", Vec::new())?;
            let text = word_text(var_to_json(&raw.0).as_str().unwrap_or(""));
            let paras = count(&doc, "Paragraphs")?;
            let words = text.split_whitespace().count();
            Ok((text, paras, words))
        })();
        let _ = doc.get("Close", vec![var_i32(0)]);
        let (text, paras, words) = r?;
        drop(w);
        let cut = text.chars().count() > max;
        let body: String = text.chars().take(max).collect();
        Ok(json!({
            "text": format!("{shown} ({paras} paragraphs, {words} words){}\n{body}", if cut { format!("; first {max} characters") } else { String::new() }),
            "paragraphs": paras,
            "words": words,
            "truncated": cut,
        }))
    })
}

pub fn word_append(a: &Value) -> Result<Value, ActError> {
    let path = crate::skills::checked(str_arg(a, "path")?)?;
    let text = a.get("text").and_then(Value::as_str).filter(|t| !t.is_empty()).ok_or_else(|| bad("'text' is required"))?.to_string();
    if text.chars().count() > 100_000 {
        return Err(bad("'text' is limited to 100,000 characters"));
    }
    let save_as = match a.get("save_as").and_then(Value::as_str) {
        Some(s) => {
            let p = crate::skills::checked(s)?;
            if p.exists() {
                return Err(bad(format!("{} already exists (never overwrites)", p.display())));
            }
            Some(p)
        }
        None => None,
    };
    let new_doc = !path.exists();
    let target = save_as.clone().unwrap_or_else(|| path.clone());
    let n = text.chars().count();
    run("Word.Application", move || {
        let w = App::start("Word.Application")?;
        let doc = if new_doc {
            w.app.obj("Documents", Vec::new())?.obj("Add", Vec::new())?
        } else {
            let d = open_doc(&w.app, &path, false)?;
            if var_to_json(&d.get("ReadOnly", Vec::new())?.0) == json!(true) && save_as.is_none() {
                let _ = d.get("Close", vec![var_i32(0)]);
                return Err(human(format!("{} is open elsewhere (Word opened it read-only)", path.display()), "Ask the user to close it, or pass save_as to write a copy."));
            }
            d
        };
        let r = (|| {
            let content = doc.obj("Content", Vec::new())?;
            let existing = var_to_json(&content.get("Text", Vec::new())?.0);
            // A new document has one empty paragraph: write into it; else start a new one.
            let body = text.replace("\r\n", "\n").replace('\n', "\r");
            let add = if existing.as_str().map(|s| s.trim().is_empty()).unwrap_or(true) { body } else { format!("\r{body}") };
            if new_doc {
                content.put("Text", var_str(&add))?;
            } else {
                content.get("InsertAfter", vec![var_str(&add)])?;
            }
            if new_doc || save_as.is_some() {
                doc.get("SaveAs2", vec![var_str(&target.to_string_lossy()), var_i32(16)])?; // wdFormatXMLDocument
            } else {
                doc.get("Save", Vec::new())?;
            }
            Ok(())
        })();
        let _ = doc.get("Close", vec![var_i32(0)]);
        r?;
        drop(w);
        Ok(json!({"text": format!("ok added {n} character(s) to {}", target.display()), "path": target.display().to_string()}))
    })
}

// ---------------------------------------------------------------------------------------------
// PowerPoint (slice 3, ADR 0062)

const MSO_TRUE: i32 = -1;
const MSO_FALSE: i32 = 0;

/// Text of a PowerPoint text range → plain lines (pure): paragraph `\r`, line break `\x0b`.
pub fn ppt_text(raw: &str) -> String {
    raw.replace('\x0b', "\n").replace('\r', "\n").trim().to_string()
}

/// The checked arguments of ppt_add_slide (pure): title, bullets.
pub fn slide_args(a: &Value) -> Result<(String, Vec<String>), ActError> {
    let title = a.get("title").and_then(Value::as_str).unwrap_or("").trim().to_string();
    let bullets: Vec<String> = match a.get("bullets") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(v)) => v
            .iter()
            .map(|b| b.as_str().map(|s| s.trim().to_string()).ok_or_else(|| bad("'bullets' must be a list of strings")))
            .collect::<Result<_, _>>()?,
        Some(_) => return Err(bad("'bullets' must be a list of strings")),
    };
    if title.is_empty() && bullets.is_empty() {
        return Err(bad("give a 'title' and/or 'bullets'"));
    }
    if bullets.len() > 50 || bullets.iter().any(|b| b.chars().count() > 2_000) || title.chars().count() > 2_000 {
        return Err(bad("at most 50 bullets of 2,000 characters each"));
    }
    Ok((title, bullets))
}

fn open_pres(app: &Disp, path: &Path, read_only: bool) -> Result<Disp, ActError> {
    // Open(FileName, ReadOnly, Untitled, WithWindow) — never shown.
    app.obj("Presentations", Vec::new())?
        .obj(
            "Open",
            vec![var_str(&path.to_string_lossy()), var_i32(if read_only { MSO_TRUE } else { MSO_FALSE }), var_i32(MSO_FALSE), var_i32(MSO_FALSE)],
        )
        .map_err(|e| human(format!("could not open {} (password-protected or damaged): {}", path.display(), e.message), "Open it in PowerPoint by hand."))
}

fn shape_text(shape: &Disp) -> Result<Option<String>, ActError> {
    if var_to_json(&shape.get("HasTextFrame", Vec::new())?.0).as_i64() != Some(MSO_TRUE as i64) {
        return Ok(None);
    }
    let raw = shape.obj("TextFrame", Vec::new())?.obj("TextRange", Vec::new())?.get("Text", Vec::new())?;
    Ok(Some(ppt_text(var_to_json(&raw.0).as_str().unwrap_or(""))).filter(|t| !t.is_empty()))
}

fn slide_texts(slide: &Disp) -> Result<(String, Vec<String>), ActError> {
    let shapes = slide.obj("Shapes", Vec::new())?;
    let has_title = var_to_json(&shapes.get("HasTitle", Vec::new())?.0).as_i64() == Some(MSO_TRUE as i64);
    let title = if has_title { shape_text(&shapes.obj("Title", Vec::new())?)?.unwrap_or_default() } else { String::new() };
    let n = var_to_json(&shapes.get("Count", Vec::new())?.0).as_i64().unwrap_or(0) as i32;
    let mut body = Vec::new();
    for i in 1..=n {
        let s = shapes.obj("Item", vec![var_i32(i)])?;
        if title_is(&s)? {
            continue; // shown as the title already
        }
        if let Some(t) = shape_text(&s)? {
            body.push(t);
        }
    }
    Ok((title, body))
}

/// Whether a shape is the slide's title placeholder (ppPlaceholderTitle 1, CenterTitle 3).
fn title_is(shape: &Disp) -> Result<bool, ActError> {
    if var_to_json(&shape.get("Type", Vec::new())?.0).as_i64() != Some(14) {
        return Ok(false); // msoPlaceholder
    }
    let t = var_to_json(&shape.obj("PlaceholderFormat", Vec::new())?.get("Type", Vec::new())?.0).as_i64();
    Ok(matches!(t, Some(1) | Some(3)))
}

fn slide_notes(slide: &Disp) -> Option<String> {
    let page = slide.obj("NotesPage", Vec::new()).ok()?;
    let shapes = page.obj("Shapes", Vec::new()).ok()?;
    let body = shapes.obj("Placeholders", Vec::new()).ok()?.obj("Item", vec![var_i32(2)]).ok()?;
    shape_text(&body).ok().flatten()
}

pub fn ppt_read(a: &Value) -> Result<Value, ActError> {
    let path = crate::skills::checked(str_arg(a, "path")?)?;
    if !path.is_file() {
        return Err(bad(format!("{} does not exist", path.display())));
    }
    let notes = a.get("notes").and_then(Value::as_bool).unwrap_or(false);
    let max = a.get("max_chars").and_then(Value::as_u64).unwrap_or(20_000).clamp(100, 200_000) as usize;
    let shown = path.display().to_string();
    run("PowerPoint.Application", move || {
        let x = App::start("PowerPoint.Application")?;
        let pres = open_pres(&x.app, &path, true)?;
        let r = (|| {
            let slides = pres.obj("Slides", Vec::new())?;
            let n = var_to_json(&slides.get("Count", Vec::new())?.0).as_i64().unwrap_or(0) as i32;
            let mut out = Vec::new();
            for i in 1..=n {
                let s = slides.obj("Item", vec![var_i32(i)])?;
                let (title, body) = slide_texts(&s)?;
                let note = if notes { slide_notes(&s) } else { None };
                out.push(json!({"slide": i, "title": title, "text": body, "notes": note}));
            }
            Ok(out)
        })();
        let _ = pres.get("Close", Vec::new());
        let slides = r?;
        drop(x);
        let mut text = String::new();
        for s in &slides {
            text.push_str(&format!("[{}] {}\n", s["slide"], s["title"].as_str().filter(|t| !t.is_empty()).unwrap_or("(no title)")));
            for t in s["text"].as_array().into_iter().flatten() {
                for line in t.as_str().unwrap_or("").lines() {
                    text.push_str(&format!("  {line}\n"));
                }
            }
            if let Some(n) = s["notes"].as_str() {
                text.push_str(&format!("  notes: {}\n", n.replace('\n', " / ")));
            }
        }
        let cut = text.chars().count() > max;
        let body: String = text.chars().take(max).collect();
        Ok(json!({
            "text": format!("{shown} ({} slides){}\n{}", slides.len(), if cut { format!("; first {max} characters") } else { String::new() }, body.trim_end()),
            "slides": slides,
            "truncated": cut,
        }))
    })
}

pub fn ppt_add_slide(a: &Value) -> Result<Value, ActError> {
    let path = crate::skills::checked(str_arg(a, "path")?)?;
    let (title, bullets) = slide_args(a)?;
    let at = a.get("at").and_then(Value::as_u64).map(|n| n.max(1) as i32);
    let save_as = match a.get("save_as").and_then(Value::as_str) {
        Some(s) => {
            let p = crate::skills::checked(s)?;
            if p.exists() {
                return Err(bad(format!("{} already exists (never overwrites)", p.display())));
            }
            Some(p)
        }
        None => None,
    };
    let new_deck = !path.exists();
    if new_deck && path.extension().map(|e| e.to_string_lossy().to_lowercase()) != Some("pptx".into()) {
        return Err(bad("a new presentation must be a .pptx path"));
    }
    let target = save_as.clone().unwrap_or_else(|| path.clone());
    run("PowerPoint.Application", move || {
        let x = App::start("PowerPoint.Application")?;
        let pres = if new_deck {
            x.app.obj("Presentations", Vec::new())?.obj("Add", vec![var_i32(MSO_FALSE)])?
        } else {
            let p = open_pres(&x.app, &path, false)?;
            if var_to_json(&p.get("ReadOnly", Vec::new())?.0).as_i64() == Some(MSO_TRUE as i64) && save_as.is_none() {
                let _ = p.get("Close", Vec::new());
                return Err(human(format!("{} is open elsewhere (PowerPoint opened it read-only)", path.display()), "Ask the user to close it, or pass save_as to write a copy."));
            }
            p
        };
        let r = (|| {
            let slides = pres.obj("Slides", Vec::new())?;
            let count = var_to_json(&slides.get("Count", Vec::new())?.0).as_i64().unwrap_or(0) as i32;
            let index = at.map(|i| i.min(count + 1)).unwrap_or(count + 1);
            // ppLayoutText = 2 (title + bullets), ppLayoutTitleOnly = 11
            let layout = if bullets.is_empty() { 11 } else { 2 };
            let slide = slides.obj("Add", vec![var_i32(index), var_i32(layout)])?;
            let ph = slide.obj("Shapes", Vec::new())?.obj("Placeholders", Vec::new())?;
            let set = |i: i32, t: &str| -> Result<(), ActError> {
                ph.obj("Item", vec![var_i32(i)])?.obj("TextFrame", Vec::new())?.obj("TextRange", Vec::new())?.put("Text", var_str(t))
            };
            set(1, &title)?;
            if !bullets.is_empty() {
                set(2, &bullets.join("\r"))?;
            }
            if new_deck || save_as.is_some() {
                pres.get("SaveAs", vec![var_str(&target.to_string_lossy()), var_i32(24)])?; // ppSaveAsOpenXMLPresentation
            } else {
                pres.get("Save", Vec::new())?;
            }
            Ok(index)
        })();
        let _ = pres.get("Close", Vec::new());
        let index = r?;
        drop(x);
        Ok(json!({"text": format!("ok added slide {index} to {}", target.display()), "path": target.display().to_string(), "slide": index}))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excel_format_args() {
        assert_eq!(format_args(&json!({})).unwrap(), (true, vec![]));
        assert_eq!(format_args(&json!({"autofit": false, "bold": ["A1:D1", " A8 "]})).unwrap(), (false, vec!["A1:D1".to_string(), "A8".to_string()]));
        assert!(format_args(&json!({"bold": "A1"})).is_err());
        assert!(format_args(&json!({"bold": ["A1;D1"]})).is_err());
        assert!(format_args(&json!({"bold": ["=SUM(A1)"]})).is_err());
        assert!(format_args(&json!({"autofit": "yes"})).is_err());
        let many: Vec<String> = (1..=21).map(|i| format!("A{i}")).collect();
        assert!(format_args(&json!({"bold": many})).is_err());
    }

    #[test]
    fn ppt_text_and_slide_args() {
        assert_eq!(ppt_text("First\rSecond\x0bsame\r"), "First\nSecond\nsame");
        let (t, b) = slide_args(&json!({"title": " Plan ", "bullets": ["a", "b"]})).unwrap();
        assert_eq!((t.as_str(), b.len()), ("Plan", 2));
        assert!(slide_args(&json!({"title": "Only"})).unwrap().1.is_empty());
        assert!(slide_args(&json!({})).is_err());
        assert!(slide_args(&json!({"title": "x", "bullets": "a"})).is_err());
        assert!(slide_args(&json!({"title": "x", "bullets": [1]})).is_err());
        let many: Vec<String> = (0..51).map(|i| i.to_string()).collect();
        assert!(slide_args(&json!({"title": "x", "bullets": many})).is_err());
    }

    #[test]
    fn word_text_becomes_lines() {
        assert_eq!(word_text("Title\rFirst line\x0bsame paragraph\r\r"), "Title\nFirst line\nsame paragraph");
        assert_eq!(word_text("a\x07b\r\x07"), "a\tb");
    }

    #[test]
    fn files_map_to_their_office_app() {
        assert_eq!(app_for(Path::new("a.DOCX")), Some("Word.Application"));
        assert_eq!(app_for(Path::new("b.xlsx")), Some("Excel.Application"));
        assert_eq!(app_for(Path::new("c.pptx")), Some("PowerPoint.Application"));
        assert_eq!(app_for(Path::new("d.pdf")), None);
        assert_eq!(pdf_target(Path::new(r"C:\x\report.docx"), None), PathBuf::from(r"C:\x\report.pdf"));
        assert_eq!(num(12.0), json!(12));
        assert_eq!(num(1.5), json!(1.5));
        assert!(write_values(&json!({"values": [["a", 1], [true, null]]})).is_ok());
        assert!(write_values(&json!({"values": [[{"x": 1}]]})).is_err());
        assert!(write_values(&json!({"values": []})).is_err());
        let big: Vec<Value> = (0..20).map(|_| json!((0..60).collect::<Vec<_>>())).collect();
        assert!(write_values(&json!({ "values": big })).is_err(), "1,200 cells > 1,000");
    }

    #[test]
    fn variants_round_trip() {
        assert_eq!(var_to_json(&var_f64(3.0).0), json!(3));
        assert_eq!(var_to_json(&var_str("Pens").0), json!("Pens"));
        assert_eq!(var_to_json(&var_bool(true).0), json!(true));
        assert_eq!(var_to_json(&Var(VARIANT::default()).0), Value::Null);
    }
}
