//! Perception + background actions (Phase 1 M4, ADRs 0014, 0015).
//!
//! - One `ElementFromHandleBuildCache` call fetches a window's whole control-view tree with the
//!   properties we print (one cross-process round trip; ADR 0003).
//! - Elements get refs `e<n>`; a re-observe of the same window reuses refs by UIA RuntimeId, so
//!   refs stay valid across calls and a diff can be returned.
//! - Actions on refs use UIA control patterns first (no cursor, no keystrokes); only when no
//!   pattern fits do they fall back to polite real input (input.rs).
//!
//! Docs:
//! UI Automation client  https://learn.microsoft.com/windows/win32/winauto/uiauto-clientsoverview
//! Caching               https://learn.microsoft.com/windows/win32/winauto/uiauto-cachingforclients
//! Control pattern ids   https://learn.microsoft.com/windows/win32/winauto/uiauto-controlpattern-ids
//! Control type ids      https://learn.microsoft.com/windows/win32/winauto/uiauto-controltype-ids
//! CoIncrementMTAUsage   https://learn.microsoft.com/windows/win32/api/combaseapi/nf-combaseapi-coincrementmtausage
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::collections::{BTreeMap, HashMap};
use std::ffi::c_void;
use std::sync::{Arc, Mutex, Once};
use std::thread::sleep;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use windows::core::{Interface, BSTR};
use windows::Win32::Foundation::{HWND as WHWND, RECT as WRECT};
use windows::Win32::System::Com::{
    CoCreateInstance, CoIncrementMTAUsage, CoInitializeEx, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED, SAFEARRAY,
};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
    SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::{VariantClear, VT_BOOL, VT_BSTR, VT_I4};
use windows::Win32::UI::Accessibility::*;

use crate::input::{self, ActError, HWND, POINT};
use crate::winmgr;

// ---------------------------------------------------------------------------------------------
// Errors

fn err(code: &'static str, message: impl Into<String>, hint: &'static str) -> ActError {
    ActError {
        code,
        message: message.into(),
        hint,
    }
}

fn bad(message: impl Into<String>) -> ActError {
    err(
        "INVALID_ARGS",
        message,
        "Check the tool schema in docs/04-mcp-tool-contract.md.",
    )
}

fn stale(r: &str) -> ActError {
    err(
        "STALE_REF",
        format!("{r} no longer exists (the element or its window changed)"),
        "Call observe again and use the new refs.",
    )
}

/// Maps a UIA/COM failure to a contract error code.
fn com_err(what: &str, e: windows::core::Error) -> ActError {
    match e.code().0 as u32 {
        0x8004_0200 => err(
            "INVALID_ARGS",
            format!("{what}: the element is disabled"),
            "Wait until it is enabled (observe shows (off)).",
        ),
        0x8004_0201 | 0x8000_4005 | 0x8001_0108 => err(
            "STALE_REF",
            format!("{what}: the element is no longer available"),
            "Call observe again and use the new refs.",
        ),
        0x8013_1505 => err(
            "TIMEOUT",
            format!("{what}: the app did not answer in time"),
            "The app may be busy or showing a modal dialog; observe its windows.",
        ),
        _ => err("INTERNAL", format!("{what}: {e}"), ""),
    }
}

// ---------------------------------------------------------------------------------------------
// COM / UIA setup

/// UIA client objects are free-threaded in the MTA; the MTA is kept alive for the whole process
/// (CoIncrementMTAUsage), so elements stored in the ref table stay valid on any worker thread.
struct Uia {
    a: IUIAutomation,
    /// Whole control view (find, read_text).
    cache: IUIAutomationCacheRequest,
    /// Control view minus offscreen elements, filtered inside UIA (observe: ADR 0003 budget).
    cache_visible: IUIAutomationCacheRequest,
    /// Element only, whole control view (deep find placeholders).
    cache_one: IUIAutomationCacheRequest,
}
// SAFETY: UIA client interfaces are agile within the MTA, which every thread that touches them
// joins (com_init) and which is pinned alive by CoIncrementMTAUsage.
unsafe impl Send for Uia {}
unsafe impl Sync for Uia {}

#[derive(Clone)]
struct El(IUIAutomationElement);
// SAFETY: see Uia.
unsafe impl Send for El {}
unsafe impl Sync for El {}

static MTA: Once = Once::new();
static UIA: Mutex<Option<Arc<Uia>>> = Mutex::new(None);

pub(crate) fn com_init() {
    MTA.call_once(|| unsafe {
        let _ = CoIncrementMTAUsage(); // cookie intentionally leaked: keep the MTA for life
    });
    // Idempotent per thread (S_FALSE when already initialised in the MTA).
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

const CACHED_PROPS: &[UIA_PROPERTY_ID] = &[
    UIA_RuntimeIdPropertyId,
    UIA_NamePropertyId,
    UIA_ControlTypePropertyId,
    UIA_BoundingRectanglePropertyId,
    UIA_IsOffscreenPropertyId,
    UIA_IsEnabledPropertyId,
    UIA_IsPasswordPropertyId,
    UIA_AutomationIdPropertyId,
    UIA_IsValuePatternAvailablePropertyId,
    UIA_ValueValuePropertyId,
    UIA_ValueIsReadOnlyPropertyId,
    UIA_IsTogglePatternAvailablePropertyId,
    UIA_ToggleToggleStatePropertyId,
    UIA_IsInvokePatternAvailablePropertyId,
    UIA_IsSelectionItemPatternAvailablePropertyId,
    UIA_SelectionItemIsSelectedPropertyId,
    UIA_IsExpandCollapsePatternAvailablePropertyId,
    UIA_ExpandCollapseExpandCollapseStatePropertyId,
    UIA_IsScrollPatternAvailablePropertyId,
    UIA_IsTextPatternAvailablePropertyId,
];

fn uia() -> Result<Arc<Uia>, ActError> {
    com_init();
    let mut g = UIA.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(u) = g.as_ref() {
        return Ok(u.clone());
    }
    let u = unsafe {
        let a: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
            .or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))
            .map_err(|e| com_err("UI Automation unavailable", e))?;
        // Bound how long a hung app can block us (IUIAutomation2, Windows 8+).
        if let Ok(a2) = a.cast::<IUIAutomation2>() {
            let _ = a2.SetConnectionTimeout(2000);
            let _ = a2.SetTransactionTimeout(5000);
        }
        let control_view = a
            .ControlViewCondition()
            .map_err(|e| com_err("ControlViewCondition", e))?;
        let mut not_offscreen = windows::Win32::System::Variant::VARIANT::default();
        (*not_offscreen.Anonymous.Anonymous).vt = VT_BOOL; // boolVal stays 0 = VARIANT_FALSE
        let onscreen = a
            .CreatePropertyCondition(UIA_IsOffscreenPropertyId, &not_offscreen)
            .map_err(|e| com_err("CreatePropertyCondition", e))?;
        let visible = a
            .CreateAndCondition(&control_view, &onscreen)
            .map_err(|e| com_err("CreateAndCondition", e))?;
        let make_scoped = |filter: &IUIAutomationCondition,
                           scope: TreeScope|
         -> Result<IUIAutomationCacheRequest, ActError> {
            let c = a
                .CreateCacheRequest()
                .map_err(|e| com_err("CreateCacheRequest", e))?;
            for &p in CACHED_PROPS {
                c.AddProperty(p).map_err(|e| com_err("AddProperty", e))?;
            }
            // Optional (Windows 10 1809+): a missing property must not break perception.
            let _ = c.AddProperty(UIA_IsDialogPropertyId);
            c.SetTreeScope(scope)
                .map_err(|e| com_err("SetTreeScope", e))?;
            c.SetTreeFilter(filter)
                .map_err(|e| com_err("SetTreeFilter", e))?;
            Ok(c)
        };
        let cache_one = make_scoped(&control_view, TreeScope_Element)?;
        let make =
            |filter: &IUIAutomationCondition| -> Result<IUIAutomationCacheRequest, ActError> {
                let c = a
                    .CreateCacheRequest()
                    .map_err(|e| com_err("CreateCacheRequest", e))?;
                for &p in CACHED_PROPS {
                    c.AddProperty(p).map_err(|e| com_err("AddProperty", e))?;
                }
                let _ = c.AddProperty(UIA_IsDialogPropertyId); // optional, see above
                c.SetTreeScope(TreeScope_Subtree)
                    .map_err(|e| com_err("SetTreeScope", e))?;
                c.SetTreeFilter(filter)
                    .map_err(|e| com_err("SetTreeFilter", e))?;
                Ok(c)
            };
        let cache = make(&control_view)?;
        let cache_visible = make(&visible)?;
        Uia {
            a,
            cache,
            cache_visible,
            cache_one,
        }
    };
    let u = Arc::new(u);
    *g = Some(u.clone());
    Ok(u)
}

// ---------------------------------------------------------------------------------------------
// Element snapshot

pub fn role_name(ct: i32) -> &'static str {
    const NAMES: [&str; 41] = [
        "Button",
        "Calendar",
        "CheckBox",
        "ComboBox",
        "Edit",
        "Hyperlink",
        "Image",
        "ListItem",
        "List",
        "Menu",
        "MenuBar",
        "MenuItem",
        "ProgressBar",
        "RadioButton",
        "ScrollBar",
        "Slider",
        "Spinner",
        "StatusBar",
        "Tab",
        "TabItem",
        "Text",
        "ToolBar",
        "ToolTip",
        "Tree",
        "TreeItem",
        "Custom",
        "Group",
        "Thumb",
        "DataGrid",
        "DataItem",
        "Document",
        "SplitButton",
        "Window",
        "Pane",
        "Header",
        "HeaderItem",
        "Table",
        "TitleBar",
        "Separator",
        "SemanticZoom",
        "AppBar",
    ];
    usize::try_from(ct - 50000)
        .ok()
        .and_then(|i| NAMES.get(i).copied())
        .unwrap_or("Element")
}

fn is_interactive_role(role: &str) -> bool {
    matches!(
        role,
        "Button"
            | "CheckBox"
            | "ComboBox"
            | "Edit"
            | "Hyperlink"
            | "ListItem"
            | "MenuItem"
            | "RadioButton"
            | "Slider"
            | "Spinner"
            | "SplitButton"
            | "TabItem"
            | "TreeItem"
            | "DataItem"
            | "Document"
            | "HeaderItem"
    )
}

/// Everything we print about one element (pure data, testable without UIA).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Info {
    pub role: &'static str,
    pub name: String,
    pub aid: String,
    pub rect: (i32, i32, i32, i32), // x, y, w, h (physical px)
    pub offscreen: bool,
    pub enabled: bool,
    pub password: bool,
    pub value: Option<String>,
    pub toggle: Option<i32>,
    pub selected: Option<bool>,
    pub expand: Option<i32>,
    pub invoke: bool,
    pub scroll: bool,
    pub text: bool,
    /// UIA IsDialog (P6-M4, ADR 0047): e.g. a browser's JavaScript alert inside its window.
    pub dialog: bool,
}

impl Info {
    /// 0 = interactive & named, 1 = interactive, 2 = named text / scroll container; None = drop.
    pub fn rank(&self) -> Option<u8> {
        if self.offscreen || self.rect.2 <= 0 || self.rect.3 <= 0 {
            return None;
        }
        let actionable = self.invoke
            || self.toggle.is_some()
            || self.selected.is_some()
            || self.expand.is_some()
            || self.value.is_some();
        let interactive = is_interactive_role(self.role) || (actionable && self.role != "Window");
        if interactive {
            return Some(if self.name.is_empty() { 1 } else { 0 });
        }
        if (self.role == "Text" || self.role == "Header") && !self.name.trim().is_empty() {
            return Some(2);
        }
        if self.scroll && matches!(self.role, "List" | "Tree" | "DataGrid" | "Table") {
            return Some(2);
        }
        None
    }

    /// `Button "Seven"`, `Edit "Search" = "abc"`, `CheckBox "Wrap" [x]` (no ref prefix).
    pub fn line(&self) -> String {
        let mut s = String::from(self.role);
        if !self.name.is_empty() {
            s.push_str(&format!(" \"{}\"", clip(&self.name, 60)));
        } else if !self.aid.is_empty()
            && self.aid.len() <= 40
            && !self.aid.chars().all(|c| c.is_ascii_digit())
        {
            s.push_str(&format!(" #{}", self.aid));
        }
        if self.password {
            s.push_str(" = ***");
        } else if let Some(v) = &self.value {
            if !(v.is_empty() && self.role == "Button") && v != &self.name {
                s.push_str(&format!(" = \"{}\"", clip(v, 60)));
            }
        }
        match self.toggle {
            Some(0) => s.push_str(" [ ]"),
            Some(1) => s.push_str(" [x]"),
            Some(2) => s.push_str(" [-]"),
            _ => {}
        }
        if self.selected == Some(true) {
            s.push_str(" (selected)");
        }
        match self.expand {
            Some(0) => s.push_str(" (collapsed)"),
            Some(1) => s.push_str(" (expanded)"),
            _ => {}
        }
        if !self.enabled {
            s.push_str(" (off)");
        }
        s
    }
}

/// One line, quotes escaped, at most `max` chars (… when cut).
pub fn clip(s: &str, max: usize) -> String {
    let one: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .replace('"', "\\\"");
    let one = one.trim();
    if one.chars().count() <= max {
        one.to_string()
    } else {
        let mut t: String = one.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}

struct Node {
    el: El,
    rid: Vec<i32>,
    info: Info,
    /// Inside a title bar or scroll bar: window chrome, never printed (close_window,
    /// window_bounds and scroll cover it) but still searchable with find.
    chrome: bool,
    /// A virtualised placeholder from ItemContainerPattern (find deep): realised when an
    /// action uses its ref (ADR 0034).
    virt: bool,
    /// Tree depth below the snapshot root (a node's subtree = the following deeper nodes).
    depth: u32,
}

/// A cached property value, decoded and freed.
enum Prop {
    Bool(bool),
    I32(i32),
    Str(String),
    Other,
}

fn cached(el: &IUIAutomationElement, id: UIA_PROPERTY_ID) -> Prop {
    unsafe {
        let Ok(mut v) = el.GetCachedPropertyValue(id) else {
            return Prop::Other;
        };
        let inner = &v.Anonymous.Anonymous;
        let out = match inner.vt {
            VT_BOOL => Prop::Bool(inner.Anonymous.boolVal.0 != 0),
            VT_I4 => Prop::I32(inner.Anonymous.lVal),
            VT_BSTR => Prop::Str(inner.Anonymous.bstrVal.to_string()),
            _ => Prop::Other, // e.g. VT_UNKNOWN = "not supported"
        };
        let _ = VariantClear(&mut v);
        out
    }
}

fn v_bool(p: Prop) -> Option<bool> {
    match p {
        Prop::Bool(b) => Some(b),
        _ => None,
    }
}

fn v_i32(p: Prop) -> Option<i32> {
    match p {
        Prop::I32(n) => Some(n),
        _ => None,
    }
}

fn v_str(p: Prop) -> Option<String> {
    match p {
        Prop::Str(s) => Some(s),
        _ => None,
    }
}

unsafe fn safearray_i32(psa: *mut SAFEARRAY) -> Vec<i32> {
    if psa.is_null() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let (Ok(lb), Ok(ub)) = (SafeArrayGetLBound(psa, 1), SafeArrayGetUBound(psa, 1)) {
        let mut data: *mut c_void = std::ptr::null_mut();
        if ub >= lb && SafeArrayAccessData(psa, &mut data).is_ok() {
            out = std::slice::from_raw_parts(data as *const i32, (ub - lb + 1) as usize).to_vec();
            let _ = SafeArrayUnaccessData(psa);
        }
    }
    let _ = SafeArrayDestroy(psa);
    out
}

fn runtime_id(el: &IUIAutomationElement) -> Vec<i32> {
    unsafe {
        el.GetRuntimeId()
            .map(|p| safearray_i32(p))
            .unwrap_or_default()
    }
}

/// Identity for elements without a RuntimeId (some MSAA-proxied list items): role + name +
/// position. If such an element moves, its ref goes stale (observe again) — never a wrong hit.
pub fn fallback_id(ct: i32, name: &str, r: (i32, i32, i32, i32)) -> Vec<i32> {
    let mut h: u32 = 0x811c_9dc5; // FNV-1a
    for b in name.bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    vec![-1, ct, h as i32, r.0, r.1, r.2, r.3]
}

/// RuntimeId, or the fallback identity from the element's current properties.
fn identity_now(el: &IUIAutomationElement) -> Vec<i32> {
    let rid = runtime_id(el);
    if !rid.is_empty() {
        return rid;
    }
    unsafe {
        let ct = el.CurrentControlType().map(|c| c.0).unwrap_or(0);
        let name = el.CurrentName().map(|b| b.to_string()).unwrap_or_default();
        let r: WRECT = el.CurrentBoundingRectangle().unwrap_or_default();
        fallback_id(
            ct,
            &name,
            (r.left, r.top, r.right - r.left, r.bottom - r.top),
        )
    }
}

fn node(el: IUIAutomationElement) -> Node {
    unsafe {
        let ct = el.CachedControlType().map(|c| c.0).unwrap_or(0);
        let r: WRECT = el.CachedBoundingRectangle().unwrap_or_default();
        let has = |id| v_bool(cached(&el, id)).unwrap_or(false);
        let password = el.CachedIsPassword().map(|b| b.as_bool()).unwrap_or(false);
        let value = if has(UIA_IsValuePatternAvailablePropertyId) {
            Some(if password {
                String::new()
            } else {
                v_str(cached(&el, UIA_ValueValuePropertyId)).unwrap_or_default()
            })
        } else {
            None
        };
        let info = Info {
            role: role_name(ct),
            name: el.CachedName().map(|b| b.to_string()).unwrap_or_default(),
            aid: el
                .CachedAutomationId()
                .map(|b| b.to_string())
                .unwrap_or_default(),
            rect: (r.left, r.top, r.right - r.left, r.bottom - r.top),
            offscreen: el.CachedIsOffscreen().map(|b| b.as_bool()).unwrap_or(false),
            enabled: el.CachedIsEnabled().map(|b| b.as_bool()).unwrap_or(true),
            password,
            value,
            toggle: if has(UIA_IsTogglePatternAvailablePropertyId) {
                v_i32(cached(&el, UIA_ToggleToggleStatePropertyId))
            } else {
                None
            },
            selected: if has(UIA_IsSelectionItemPatternAvailablePropertyId) {
                Some(v_bool(cached(&el, UIA_SelectionItemIsSelectedPropertyId)).unwrap_or(false))
            } else {
                None
            },
            expand: if has(UIA_IsExpandCollapsePatternAvailablePropertyId) {
                v_i32(cached(&el, UIA_ExpandCollapseExpandCollapseStatePropertyId))
                    .filter(|&s| s != 3) // LeafNode: nothing to expand
            } else {
                None
            },
            invoke: has(UIA_IsInvokePatternAvailablePropertyId),
            scroll: has(UIA_IsScrollPatternAvailablePropertyId),
            text: has(UIA_IsTextPatternAvailablePropertyId),
            dialog: has(UIA_IsDialogPropertyId),
        };
        let mut rid = runtime_id(&el);
        if rid.is_empty() {
            rid = fallback_id(ct, &info.name, info.rect);
        }
        Node {
            el: El(el),
            rid,
            info,
            chrome: false,
            virt: false,
            depth: 0,
        }
    }
}

const MAX_NODES: usize = 6000;

fn collect(el: &IUIAutomationElement, depth: u32, chrome: bool, out: &mut Vec<Node>) {
    if out.len() >= MAX_NODES || depth > 64 {
        return;
    }
    let mut n = node(el.clone());
    let chrome = chrome || matches!(n.info.role, "TitleBar" | "ScrollBar");
    n.chrome = chrome;
    n.depth = depth;
    out.push(n);
    let Ok(kids) = (unsafe { el.GetCachedChildren() }) else {
        return;
    };
    let n = unsafe { kids.Length() }.unwrap_or(0);
    for i in 0..n {
        if let Ok(c) = unsafe { kids.GetElement(i) } {
            collect(&c, depth + 1, chrome, out);
        }
    }
}

/// Whole cached tree of a top-level window (one cross-process round trip). Fetching the
/// top-level parts in parallel was measured (ADR 0035) and is not faster: providers answer
/// one request at a time.
fn snapshot(h: HWND, visible_only: bool) -> Result<Vec<Node>, ActError> {
    let u = uia()?;
    let cache = if visible_only {
        &u.cache_visible
    } else {
        &u.cache
    };
    let root = unsafe { u.a.ElementFromHandleBuildCache(WHWND(h), cache) }
        .map_err(|e| com_err("reading the window's UI tree", e))?;
    let mut out = Vec::with_capacity(256);
    collect(&root, 0, false, &mut out);
    Ok(out)
}

// P6-M5 (ADR 0048): Chromium (Chrome, Edge, Electron) builds its page tree only after the first
// UIA request and only while the window is not minimized.

const SETTLE_MAX: Duration = Duration::from_millis(1500);
const SETTLE_STEP: Duration = Duration::from_millis(150);

/// Chromium windows that had no page after a full wait: not waited for again.
static NO_PAGE: Mutex<Option<std::collections::HashSet<usize>>> = Mutex::new(None);

fn has_document(nodes: &[Node]) -> bool {
    nodes.iter().skip(1).any(|n| n.info.role == "Document")
}

/// Why a Chromium window shows no page: `None` (it has one, or it is not Chromium).
pub fn page_state(chromium: bool, has_doc: bool, minimized: bool) -> Option<&'static str> {
    match (chromium && !has_doc, minimized) {
        (false, _) => None,
        (true, true) => Some("minimized"),
        (true, false) => Some("not_exposed"),
    }
}

/// The footer note for `page_state` (pure, tested).
pub fn page_note(state: &str) -> &'static str {
    match state {
        "minimized" => "; page hidden: this Chromium window is minimized and has not built its page — observe with reveal:true (shows it a moment, no focus change)",
        _ => "; no page content exposed yet — focus_window once, or screenshot / ocr",
    }
}

/// `snapshot`, waiting (≤ 1.5 s) for a Chromium window's page on its first read.
/// Returns the nodes and the time waited.
fn snapshot_settled(h: HWND, visible_only: bool) -> Result<(Vec<Node>, u64), ActError> {
    let t0 = Instant::now();
    let mut nodes = snapshot(h, visible_only)?;
    if has_document(&nodes) {
        page_seen(h);
        return Ok((nodes, 0));
    }
    if !winmgr::is_chromium(h) {
        return Ok((nodes, 0));
    }
    let key = h as usize;
    let skip = || {
        NO_PAGE
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|s| s.contains(&key))
    };
    if winmgr::is_minimized(h) || skip() {
        return Ok((nodes, 0));
    }
    while t0.elapsed() < SETTLE_MAX {
        sleep(SETTLE_STEP);
        crate::safety::check()?;
        nodes = snapshot(h, visible_only)?;
        if has_document(&nodes) {
            return Ok((nodes, t0.elapsed().as_millis() as u64));
        }
    }
    NO_PAGE
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_or_insert_with(Default::default)
        .insert(key);
    Ok((nodes, t0.elapsed().as_millis() as u64))
}

/// A page appeared: wait again next time the window is empty.
fn page_seen(h: HWND) {
    if let Some(s) = NO_PAGE.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        s.remove(&(h as usize));
    }
}

/// Cached subtree of one element (observe scope_ref, P2-M1).
fn snapshot_from(el: &IUIAutomationElement) -> Result<Vec<Node>, ActError> {
    let u = uia()?;
    let root = unsafe { el.BuildUpdatedCache(&u.cache_visible) }
        .map_err(|e| com_err("reading the element's subtree", e))?;
    let mut out = Vec::with_capacity(64);
    collect(&root, 0, false, &mut out);
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Ref table

struct Entry {
    win: usize,
    el: El,
    rid: Vec<i32>,
    password: bool,
    name: String,
    virt: bool,
    role: &'static str,
}

#[derive(Default)]
struct Snap {
    rid_ref: HashMap<Vec<i32>, u32>,
    /// What the last `observe` printed: ref → line (for diffs).
    lines: BTreeMap<u32, String>,
    header: String,
    seq: u64,
}

#[derive(Default)]
struct Table {
    next: u32,
    seq: u64,
    refs: HashMap<u32, Entry>,
    snaps: HashMap<usize, Snap>,
}

const MAX_REFS: usize = 4096;

static TABLE: Mutex<Option<Table>> = Mutex::new(None);

fn with_table<T>(f: impl FnOnce(&mut Table) -> T) -> T {
    let mut g = TABLE.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Table::default))
}

impl Table {
    /// Ref for this element (reused when the window already knew its RuntimeId).
    fn ref_for(&mut self, win: usize, n: &Node) -> u32 {
        let snap = self.snaps.entry(win).or_default();
        if let Some(&r) = snap.rid_ref.get(&n.rid) {
            if let Some(e) = self.refs.get_mut(&r) {
                e.el = n.el.clone(); // fresher element object, same identity
                e.password = n.info.password;
                e.name = n.info.name.clone();
                return r;
            }
        }
        self.next += 1;
        let r = self.next;
        snap.rid_ref.insert(n.rid.clone(), r);
        self.refs.insert(
            r,
            Entry {
                win,
                el: n.el.clone(),
                rid: n.rid.clone(),
                password: n.info.password,
                name: n.info.name.clone(),
                virt: n.virt,
                role: n.info.role,
            },
        );
        r
    }

    fn touch(&mut self, win: usize) {
        self.seq += 1;
        let seq = self.seq;
        self.snaps.entry(win).or_default().seq = seq;
        // Evict whole windows (oldest first) when the table grows too large.
        while self.refs.len() > MAX_REFS && self.snaps.len() > 1 {
            let Some((&old, _)) = self
                .snaps
                .iter()
                .filter(|(&w, _)| w != win)
                .min_by_key(|(_, s)| s.seq)
            else {
                break;
            };
            self.forget_window(old);
        }
    }

    fn forget_window(&mut self, win: usize) {
        if let Some(s) = self.snaps.remove(&win) {
            for r in s.rid_ref.values() {
                self.refs.remove(r);
            }
        }
    }
}

/// The admin helper numbers its refs from here (ADR 0030), so refs never collide and the core
/// knows which process owns one.
pub const HELPER_REF_BASE: u32 = 50_000_000;

/// Start numbering refs at `base` (admin helper process only).
pub fn set_ref_base(base: u32) {
    with_table(|t| t.next = t.next.max(base));
}

pub fn parse_elem_ref(v: &Value) -> Option<u32> {
    v.as_str()
        .and_then(|s| s.strip_prefix('e'))
        .and_then(|d| d.parse::<u32>().ok())
        .filter(|&n| n != 0)
}

/// What the policy needs to know about a ref, from the table (no UIA call): window, name,
/// password flag. None for unknown refs (the action itself reports STALE_REF).
pub fn ref_facts(v: &Value) -> Option<(HWND, String, bool)> {
    let n = parse_elem_ref(v)?;
    with_table(|t| {
        t.refs
            .get(&n)
            .map(|e| (e.win as HWND, e.name.clone(), e.password))
    })
}

/// Window, role, name and password flag of a ref (record_session locators, ADR 0036).
pub fn ref_locator(v: &Value) -> Option<(HWND, &'static str, String, bool)> {
    let n = parse_elem_ref(v)?;
    with_table(|t| {
        t.refs
            .get(&n)
            .map(|e| (e.win as HWND, e.role, e.name.clone(), e.password))
    })
}

/// A ref for the element with exactly this role and name in a window (replay of recorded
/// steps). Visible elements win over off-screen ones; none → ELEMENT_NOT_FOUND.
pub fn locate_element(h: HWND, role: &str, name: &str) -> Result<String, ActError> {
    let nodes = snapshot(h, false)?;
    let mut hits: Vec<&Node> = nodes
        .iter()
        .skip(1)
        .filter(|n| n.info.role == role && n.info.name == name)
        .collect();
    hits.sort_by_key(|n| n.info.offscreen || (n.info.rect.2 <= 0 && n.info.rect.3 <= 0));
    let n = hits.first().ok_or_else(|| {
        err(
            "ELEMENT_NOT_FOUND",
            format!("no {role} \"{name}\" in {}", winmgr::window_ref(h)),
            "The recorded element is not there now (different screen or state).",
        )
    })?;
    let win = h as usize;
    let r = with_table(|t| {
        let r = t.ref_for(win, n);
        t.touch(win);
        r
    });
    Ok(format!("e{r}"))
}

/// Name and password flag of the element under a screen point (policy for real clicks).
pub fn element_at(p: POINT) -> Option<(String, bool)> {
    let u = uia().ok()?;
    unsafe {
        let el =
            u.a.ElementFromPoint(windows::Win32::Foundation::POINT { x: p.x, y: p.y })
                .ok()?;
        Some((
            el.CurrentName().map(|b| b.to_string()).unwrap_or_default(),
            el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(false),
        ))
    }
}

/// The row-like item (ListItem, DataItem, TreeItem) under a screen point, if any: (role,
/// name). `drag` uses it so nothing is dropped onto another item by accident: a file dropped
/// on a folder icon moves into it, on the Recycle Bin it is deleted.
pub fn item_at(p: POINT) -> Option<(&'static str, String)> {
    let u = uia().ok()?;
    unsafe {
        let mut el =
            u.a.ElementFromPoint(windows::Win32::Foundation::POINT { x: p.x, y: p.y })
                .ok()?;
        let walker = u.a.ControlViewWalker().ok()?;
        for _ in 0..4 {
            let role = role_name(el.CurrentControlType().map(|c| c.0).unwrap_or(0));
            if ROW_ROLES.contains(&role) {
                return Some((
                    role,
                    el.CurrentName().map(|b| b.to_string()).unwrap_or_default(),
                ));
            }
            el = walker.GetParentElement(&el).ok()?;
        }
    }
    None
}

/// True if the element with keyboard focus is a password field (typing there is denied).
pub fn focused_is_password() -> bool {
    let Ok(u) = uia() else {
        return false;
    };
    unsafe {
        u.a.GetFocusedElement()
            .and_then(|e| e.CurrentIsPassword())
            .map(|b| b.as_bool())
            .unwrap_or(false)
    }
}

/// UTF-16 length of the focused field's value (None: no value, or a password field). Lets
/// `type` return only once the app has really taken the text (M6 bench finding).
pub fn focused_value_len() -> Option<usize> {
    let u = uia().ok()?;
    unsafe {
        let el = u.a.GetFocusedElement().ok()?;
        if el.CurrentIsPassword().map(|b| b.as_bool()).unwrap_or(true) {
            return None;
        }
        let vp = el
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()?;
        vp.CurrentValue().ok().map(|b| b.len())
    }
}

/// A live element behind a ref, verified to still be the same element.
pub struct Target {
    el: El,
    pub window: HWND,
    pub password: bool,
}

pub fn resolve(v: &Value) -> Result<Target, ActError> {
    let n = parse_elem_ref(v).ok_or_else(|| bad("'ref' must look like \"e12\" (from observe)"))?;
    let name = format!("e{n}");
    let (win, el, rid, password, virt) = with_table(|t| {
        t.refs
            .get(&n)
            .map(|e| (e.win, e.el.clone(), e.rid.clone(), e.password, e.virt))
    })
    .ok_or_else(|| stale(&name))?;
    let window = win as HWND;
    if !winmgr::exists(window) {
        with_table(|t| t.forget_window(win));
        return Err(stale(&name));
    }
    if winmgr::is_elevated(window) {
        return Err(winmgr::elevation_required("this element"));
    }
    uia()?;
    if virt {
        // A virtualised row found with find(deep): make it real now (this may scroll the
        // list to it), then act on it.
        unsafe {
            if let Ok(vp) =
                el.0.GetCurrentPatternAs::<IUIAutomationVirtualizedItemPattern>(
                    UIA_VirtualizedItemPatternId,
                )
            {
                vp.Realize()
                    .map_err(|e| com_err("realizing the list row", e))?;
            }
        }
        return Ok(Target {
            el,
            window,
            password,
        });
    }
    // Alive and still the same element?
    let pid = unsafe { el.0.CurrentProcessId() };
    let now = identity_now(&el.0);
    if pid.is_err() || now != rid {
        return Err(stale(&name));
    }
    Ok(Target {
        el,
        window,
        password,
    })
}

/// The ref's element is a password field (typing there is denied).
pub fn is_password(t: &Target) -> bool {
    t.password
        || unsafe { t.el.0.CurrentIsPassword() }
            .map(|b| b.as_bool())
            .unwrap_or(false)
}

/// Keyboard focus to the ref's element (`type {ref}`, P6-M6). The window is already in front.
pub fn focus_element(t: &Target) -> Result<(), ActError> {
    unsafe { t.el.0.SetFocus() }.map_err(|e| com_err("focusing the field", e))
}

impl Target {
    pub fn center(&self) -> Result<POINT, ActError> {
        let r = unsafe { self.el.0.CurrentBoundingRectangle() }
            .map_err(|e| com_err("reading the element position", e))?;
        if r.right <= r.left || r.bottom <= r.top {
            return Err(err(
                "TARGET_OBSCURED",
                "the element has no visible area (offscreen or collapsed)",
                "Scroll it into view (scroll with ref) or observe again.",
            ));
        }
        Ok(POINT {
            x: (r.left + r.right) / 2,
            y: (r.top + r.bottom) / 2,
        })
    }

    fn pattern<T: Interface>(&self, id: UIA_PATTERN_ID) -> Option<T> {
        unsafe { self.el.0.GetCurrentPatternAs::<T>(id) }.ok()
    }

    fn enabled(&self) -> bool {
        unsafe { self.el.0.CurrentIsEnabled() }
            .map(|b| b.as_bool())
            .unwrap_or(true)
    }
}

// ---------------------------------------------------------------------------------------------
// Methods

pub const METHODS: &[&str] = &[
    "observe",
    "find",
    "read_text",
    "set_value",
    "screenshot",
    "wait_for",
    "ocr",
    "frame_diff",
];

pub fn run(method: &str, p: &Value) -> Result<Value, ActError> {
    input::init_dpi();
    crate::safety::check()?;
    match method {
        "observe" => observe(p),
        "find" => find(p),
        "read_text" => read_text(p),
        "set_value" => {
            let _one = input::action_lock();
            set_value(p)
        }
        "screenshot" => crate::capture::screenshot(p),
        "wait_for" => wait_for(p),
        "ocr" => crate::ocr::ocr(p),
        "frame_diff" => crate::framediff::frame_diff(p),
        m => Err(bad(format!("'{m}' is not a perception method"))),
    }
}

fn target_window(p: &Value) -> Result<HWND, ActError> {
    let h = match p.get("window") {
        None | Some(Value::Null) => {
            let fg = winmgr::foreground_root();
            if fg.is_null() {
                return Err(err(
                    "WINDOW_NOT_FOUND",
                    "no foreground window",
                    "Pass 'window' (from list_windows).",
                ));
            }
            fg
        }
        Some(v) => winmgr::resolve_window(v)?,
    };
    if crate::safety::is_own_window(h) {
        return Err(crate::safety::blocked(
            "that is an XtraOS window (overlay/dialog): only the user may use it",
        ));
    }
    if winmgr::is_elevated(h) {
        return Err(winmgr::elevation_required("reading this window"));
    }
    Ok(h)
}

fn header(h: HWND) -> String {
    let f = winmgr::frame(h);
    format!(
        "{} \"{}\" {} {}x{}",
        winmgr::window_ref(h),
        clip(&winmgr::title(h), 80),
        winmgr::process_name(winmgr::pid_of(h)),
        f.right - f.left,
        f.bottom - f.top
    )
}

/// Picks at most `max` nodes by rank, keeping tree order. Returns (indices, dropped count).
pub fn select(ranks: &[Option<u8>], max: usize) -> (Vec<usize>, usize) {
    let mut idx: Vec<usize> = (0..ranks.len()).filter(|&i| ranks[i].is_some()).collect();
    let total = idx.len();
    if total > max {
        let mut by_rank = idx.clone();
        by_rank.sort_by_key(|&i| (ranks[i].unwrap(), i));
        by_rank.truncate(max);
        by_rank.sort_unstable();
        idx = by_rank;
    }
    (idx, total.saturating_sub(max))
}

/// Diff between two ref→line maps: (changed, added, removed) lines, ready to print.
pub fn diff(
    old: &BTreeMap<u32, String>,
    new: &BTreeMap<u32, String>,
) -> (Vec<String>, usize, usize, usize) {
    let mut out = Vec::new();
    let (mut c, mut a, mut r) = (0, 0, 0);
    for (k, l) in new {
        match old.get(k) {
            Some(o) if o == l => {}
            Some(_) => {
                c += 1;
                out.push(format!("~ e{k} {l}"));
            }
            None => {
                a += 1;
                out.push(format!("+ e{k} {l}"));
            }
        }
    }
    for (k, l) in old {
        if !new.contains_key(k) {
            r += 1;
            out.push(format!("- e{k} {l}"));
        }
    }
    (out, c, a, r)
}

// ---------------------------------------------------------------------------------------------
// Dialogs (P6-M4, ADR 0047)

/// The first visible IsDialog element below the root: the node indices of its subtree
/// (`start - 1` is the dialog itself).
fn in_tree_dialog(nodes: &[Node]) -> Option<std::ops::Range<usize>> {
    let i = nodes.iter().enumerate().skip(1).find_map(|(i, n)| {
        (n.info.dialog && !n.info.offscreen && n.info.rect.2 > 0 && n.info.rect.3 > 0).then_some(i)
    })?;
    let d = nodes[i].depth;
    let end = nodes[i + 1..]
        .iter()
        .position(|n| n.depth <= d)
        .map_or(nodes.len(), |p| i + 1 + p);
    Some(i + 1..end)
}

/// The dialog's message (its texts, minus the title) and its buttons with refs.
fn dialog_parts(
    t: &mut Table,
    win: usize,
    nodes: &[Node],
    range: std::ops::Range<usize>,
    title: &str,
) -> (String, Vec<(String, String)>) {
    let mut texts: Vec<String> = Vec::new();
    let mut buttons = Vec::new();
    for n in &nodes[range] {
        if n.chrome || n.info.rank().is_none() {
            continue;
        }
        match n.info.role {
            "Text" if !n.info.name.trim().is_empty() && n.info.name.trim() != title.trim() => {
                texts.push(n.info.name.trim().to_string())
            }
            "Button" | "SplitButton" => {
                buttons.push((format!("e{}", t.ref_for(win, n)), n.info.name.clone()))
            }
            _ => {}
        }
    }
    t.touch(win);
    (clip(&texts.join(" "), 200), buttons)
}

/// `!! dialog e45 "This page says": "Saved" — buttons: e46 "OK". …` (pure, tested).
pub fn dialog_line(
    r: &str,
    title: &str,
    msg: &str,
    buttons: &[(String, String)],
    blocking: bool,
    elevated: bool,
) -> String {
    let mut s = format!("!! dialog {r} \"{}\"", clip(title, 60));
    if blocking {
        s.push_str(" (blocking)");
    }
    if elevated {
        s.push_str(": an administrator dialog — reading it needs elevated_helper {op:\"start\"}, or ask the user");
        return s;
    }
    if !msg.is_empty() {
        s.push_str(&format!(": \"{msg}\""));
    }
    if buttons.is_empty() {
        s.push_str(" — no buttons found: observe its window, or press_keys Enter/Escape.");
    } else {
        let b: Vec<String> = buttons
            .iter()
            .map(|(r, n)| format!("{r} \"{}\"", clip(n, 30)))
            .collect();
        s.push_str(&format!(" — buttons: {}.", b.join(", ")));
    }
    s.push_str(" Answer it first; the window may not respond until it closes.");
    s
}

fn dialog_value(
    r: &str,
    title: &str,
    msg: &str,
    buttons: &[(String, String)],
    kind: &str,
    blocking: bool,
) -> Value {
    json!({
        "ref": r,
        "title": title,
        "text": msg,
        "buttons": buttons.iter().map(|(r, n)| json!({"ref": r, "name": n})).collect::<Vec<_>>(),
        "kind": kind,
        "blocking": blocking,
    })
}

fn observe(p: &Value) -> Result<Value, ActError> {
    if let Some(r) = p.get("scope_ref").filter(|v| !v.is_null()) {
        return observe_scope(p, r);
    }
    let t0 = Instant::now();
    let h = target_window(p)?;
    // `bounds`: each line also gets its on-screen place, `@cx,cy (x,y wxh)` (as ocr), e.g. to
    // plan where to drag desktop icons. Always the full list (positions are not diffed).
    let bounds = p.get("bounds").and_then(Value::as_bool).unwrap_or(false);
    let full = bounds || p.get("full").and_then(Value::as_bool).unwrap_or(false);
    let max = match p.get("max_elements") {
        None | Some(Value::Null) => 120,
        Some(v) => v
            .as_u64()
            .filter(|n| (1..=500).contains(n))
            .ok_or_else(|| bad("'max_elements' must be 1..=500"))? as usize,
    };
    let reveal = p.get("reveal").and_then(Value::as_bool).unwrap_or(false);
    let ts = Instant::now();
    // P6-M5 (ADR 0048): wait for a Chromium page on the first read; `reveal` shows a minimized
    // Chromium window for a moment (no activation) so it builds its page, then re-minimizes it.
    let (mut nodes, mut waited_ms) = snapshot_settled(h, true)?;
    let chromium = winmgr::is_chromium(h);
    let mut revealed = false;
    if reveal && chromium && !has_document(&nodes) && winmgr::is_minimized(h) {
        let _shown = winmgr::reveal_no_activate(h); // minimized again when dropped
        revealed = true;
        let t1 = Instant::now();
        while t1.elapsed() < SETTLE_MAX {
            sleep(SETTLE_STEP);
            crate::safety::check()?;
            nodes = snapshot(h, true)?;
            if has_document(&nodes) {
                page_seen(h);
                break;
            }
        }
        waited_ms += t1.elapsed().as_millis() as u64;
    }
    let page = page_state(chromium, has_document(&nodes), winmgr::is_minimized(h));
    let snapshot_ms = ts.elapsed().as_millis() as u64;
    // P6-M4 (ADR 0047): a dialog inside the window (IsDialog, e.g. a browser alert) and its
    // subtree are always kept; a dialog window owned by this one is read separately.
    let in_tree = in_tree_dialog(&nodes);
    let owned = winmgr::owned_dialog(h).map(|(d, blocking)| {
        let elevated = winmgr::is_elevated(d);
        let dn = if elevated {
            None
        } else {
            snapshot(d, true).ok()
        };
        (d, blocking, elevated, dn)
    });
    let ranks: Vec<Option<u8>> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| {
            if i == 0 || n.chrome {
                None
            } else {
                let boost = in_tree.as_ref().is_some_and(|r| r.contains(&i));
                n.info
                    .rank()
                    .map(|r| if boost { 0 } else { r.saturating_add(1) })
            }
        })
        .collect();
    let (keep, dropped) = select(&ranks, max);
    let f = winmgr::frame(h);
    let area = (f.right - f.left).max(0) as i64 * (f.bottom - f.top).max(0) as i64;
    let opaque = area > 200 * 200 && keep.len() < 5;
    let mut head = header(h);
    if let Some(o) = winmgr::dialog_owner(h) {
        head.push_str(&format!(" dialog of {}", winmgr::window_ref(o)));
    }
    if opaque {
        head.push_str(" opaque");
    }
    let win = h as usize;
    let owned_title = owned.as_ref().map(|(d, ..)| winmgr::title(*d));
    let owned_ref = owned.as_ref().map(|(d, ..)| winmgr::window_ref(*d));

    let mut dialog_json = Value::Null;
    let mut dialog_line_text: Option<String> = None;
    let (text, is_diff) = with_table(|t| {
        // The dialog report (refs come from the same table as every other element).
        if let (Some((d, blocking, elevated, dn)), Some(title), Some(r)) =
            (owned.as_ref(), owned_title.as_ref(), owned_ref.as_ref())
        {
            let (msg, buttons) = match dn {
                Some(dn) if dn.len() > 1 => dialog_parts(t, *d as usize, dn, 1..dn.len(), title),
                _ => (String::new(), Vec::new()),
            };
            dialog_line_text = Some(dialog_line(r, title, &msg, &buttons, *blocking, *elevated));
            dialog_json = dialog_value(r, title, &msg, &buttons, "window", *blocking);
        } else if let Some(range) = in_tree.clone() {
            let i = range.start - 1;
            let r = format!("e{}", t.ref_for(win, &nodes[i]));
            let title = nodes[i].info.name.clone();
            let (msg, buttons) = dialog_parts(t, win, &nodes, range, &title);
            dialog_line_text = Some(dialog_line(&r, &title, &msg, &buttons, true, false));
            dialog_json = dialog_value(&r, &title, &msg, &buttons, "in_page", true);
        }

        let mut lines = BTreeMap::new();
        let mut ordered = Vec::with_capacity(keep.len());
        for &i in &keep {
            let r = t.ref_for(win, &nodes[i]);
            let l = nodes[i].info.line();
            if bounds {
                let (x, y, w, h) = nodes[i].info.rect;
                ordered.push(format!(
                    "e{r} {l} @{},{} ({x},{y} {w}x{h})",
                    x + w / 2,
                    y + h / 2
                ));
            } else {
                ordered.push(format!("e{r} {l}"));
            }
            lines.insert(r, l);
        }
        t.touch(win);
        let snap = t.snaps.get_mut(&win).expect("touched");
        let had_baseline = !snap.lines.is_empty() || !snap.header.is_empty();
        let mut footer = format!("-- {} elements", keep.len());
        if dropped > 0 {
            footer.push_str(&format!(" (+{dropped} more; raise max_elements)"));
        }
        // P6-M5: a missing Chromium page says why (it is not "opaque", it is not built yet).
        let note = page.map(page_note).unwrap_or("");
        if !note.is_empty() {
            footer.push_str(note);
        } else if opaque {
            footer.push_str("; opaque: few accessible elements — use screenshot");
        }
        let full_text = format!("{head}\n{}\n{footer}", ordered.join("\n"));
        let mut result = (full_text.clone(), false);
        if had_baseline && !full {
            let (d, c, a, r) = diff(&snap.lines, &lines);
            let mut parts = Vec::new();
            if snap.header != head {
                parts.push(head.clone());
            }
            parts.extend(d);
            let summary = if parts.is_empty() {
                format!("-- no changes{note}")
            } else {
                format!("-- diff: {c} changed, {a} added, {r} removed{note}")
            };
            let diff_text = if parts.is_empty() {
                format!("{}\n{summary}", winmgr::window_ref(h))
            } else {
                format!("{}\n{summary}", parts.join("\n"))
            };
            if diff_text.len() < full_text.len() {
                result = (diff_text, true);
            }
        }
        snap.lines = lines;
        snap.header = head.clone();
        // The dialog goes right after the first line, in the full list and in a diff.
        if let Some(d) = &dialog_line_text {
            let (first, rest) = result.0.split_once('\n').unwrap_or((&result.0, ""));
            result.0 = format!("{first}\n{d}\n{rest}");
        }
        result
    });
    Ok(json!({
        "text": text,
        "window": winmgr::window_ref(h),
        "elements": keep.len(),
        "opaque": opaque,
        "dialog": dialog_json,
        "page": page,
        "waited_ms": waited_ms,
        "revealed": revealed,
        "diff": is_diff,
        "ms": t0.elapsed().as_millis() as u64,
        "snapshot_ms": snapshot_ms,
    }))
}

fn max_elements(p: &Value) -> Result<usize, ActError> {
    match p.get("max_elements") {
        None | Some(Value::Null) => Ok(120),
        Some(v) => Ok(v
            .as_u64()
            .filter(|n| (1..=500).contains(n))
            .ok_or_else(|| bad("'max_elements' must be 1..=500"))? as usize),
    }
}

/// observe(scope_ref): only one element's subtree (a list, pane, dialog); always the full
/// list for that scope (no diff, the window's diff baseline is untouched).
fn observe_scope(p: &Value, r: &Value) -> Result<Value, ActError> {
    let t0 = Instant::now();
    let t = resolve(r)?;
    let max = max_elements(p)?;
    let nodes = snapshot_from(&t.el.0)?;
    let ranks: Vec<Option<u8>> = nodes
        .iter()
        .map(|n| if n.chrome { None } else { n.info.rank() })
        .collect();
    let (keep, dropped) = select(&ranks, max);
    let win = t.window as usize;
    let scope_line = nodes.first().map(|n| n.info.line()).unwrap_or_default();
    let text = with_table(|tab| {
        let mut out = Vec::with_capacity(keep.len());
        for &i in &keep {
            let rf = tab.ref_for(win, &nodes[i]);
            out.push(format!("e{rf} {}", nodes[i].info.line()));
        }
        tab.touch(win);
        let mut footer = format!("-- {} elements in scope", keep.len());
        if dropped > 0 {
            footer.push_str(&format!(" (+{dropped} more; raise max_elements)"));
        }
        format!(
            "{} scope {} {}\n{}\n{footer}",
            winmgr::window_ref(t.window),
            r.as_str().unwrap_or("?"),
            scope_line,
            out.join("\n")
        )
    });
    Ok(json!({
        "text": text,
        "window": winmgr::window_ref(t.window),
        "elements": keep.len(),
        "opaque": false,
        "diff": false,
        "ms": t0.elapsed().as_millis() as u64,
    }))
}

fn find(p: &Value) -> Result<Value, ActError> {
    let q = p
        .get("query")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad("'query' (non-empty text) is required"))?;
    let deep = match p.get("deep") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(bad("'deep' must be true or false")),
    };
    let budget = match p.get("timeout_ms") {
        None | Some(Value::Null) => DEEP_BUDGET,
        Some(v) => Duration::from_millis(
            v.as_u64()
                .filter(|&n| n <= 15_000)
                .ok_or_else(|| bad("'timeout_ms' must be 0..=15000"))?,
        ),
    };
    let h = target_window(p)?;
    let mut nodes = snapshot_settled(h, false)?.0; // P6-M5: Chromium page on first read
    let mut deep_note = String::new();
    if deep {
        let orig = p["query"].as_str().unwrap_or("").trim().to_string();
        let t0 = Instant::now();
        let (extra, scanned, exact) = deep_items(&nodes, &orig, &q, budget);
        deep_note = if exact {
            "; deep: exact name".to_string()
        } else {
            format!(
                "; deep: scanned {scanned} rows in {} ms{}",
                t0.elapsed().as_millis(),
                if t0.elapsed() >= budget {
                    " (time budget reached: raise timeout_ms or use the exact name)"
                } else {
                    ""
                }
            )
        };
        nodes.extend(extra);
    }
    let win = h as usize;
    let mut out = Vec::new();
    let mut total = 0;
    with_table(|t| {
        for n in nodes.iter().skip(1) {
            let i = &n.info;
            let empty = i.rect.2 <= 0 && i.rect.3 <= 0;
            // Rows scrolled out of a list have no rectangle but are real rows (ADR 0034).
            if empty && !n.virt && !ROW_ROLES.contains(&i.role) {
                continue;
            }
            let hit = i.name.to_lowercase().contains(&q)
                || i.role.to_lowercase() == q
                || (!i.aid.is_empty() && i.aid.to_lowercase().contains(&q));
            if hit {
                total += 1;
                if out.len() < 20 {
                    let r = t.ref_for(win, n);
                    let mut l = format!("e{r} {}", i.line());
                    if n.virt {
                        l.push_str(" (virtualized: realized when used)");
                    } else if i.offscreen || empty {
                        l.push_str(" (offscreen)");
                    }
                    out.push(l);
                }
            }
        }
        t.touch(win);
    });
    let mut text = out.join("\n");
    text.push_str(&format!(
        "\n-- {total} matches in {}{}{deep_note}",
        winmgr::window_ref(h),
        if total > 20 { " (first 20)" } else { "" }
    ));
    Ok(json!({"text": text.trim_start(), "matches": total}))
}

const ROW_ROLES: &[&str] = &["ListItem", "DataItem", "TreeItem"];
const CONTAINER_ROLES: &[&str] = &["List", "DataGrid", "Tree", "Table"];
const DEEP_BUDGET: Duration = Duration::from_secs(3);
const DEEP_MAX_ITEMS: usize = 20_000;

/// Rows of virtualised containers that are not in the UIA tree yet (find deep, ADR 0034):
/// ItemContainerPattern lookup by exact name first, then a bounded walk over the items
/// comparing names. Placeholders are not realised here (no scrolling during a find).
fn deep_items(nodes: &[Node], orig: &str, q: &str, budget: Duration) -> (Vec<Node>, usize, bool) {
    use windows::Win32::System::Variant::{VariantClear, VARIANT, VT_BSTR};
    let Ok(u) = uia() else {
        return (Vec::new(), 0, false);
    };
    let known: std::collections::HashSet<Vec<i32>> = nodes.iter().map(|n| n.rid.clone()).collect();
    let mut seen: std::collections::HashSet<String> = nodes
        .iter()
        .filter(|n| ROW_ROLES.contains(&n.info.role))
        .map(|n| n.info.name.to_lowercase())
        .collect();
    let deadline = Instant::now() + budget;
    let mut scanned = 0usize;
    let mut out: Vec<Node> = Vec::new();
    let mut add = |e: &IUIAutomationElement, out: &mut Vec<Node>| {
        let Ok(c) = (unsafe { e.BuildUpdatedCache(&u.cache_one) }) else {
            return;
        };
        let mut n = node(c);
        if known.contains(&n.rid) || !seen.insert(n.info.name.to_lowercase()) {
            return;
        }
        n.virt = true;
        out.push(n);
    };
    for c in nodes
        .iter()
        .filter(|n| CONTAINER_ROLES.contains(&n.info.role))
    {
        let Ok(ic) = (unsafe {
            c.el.0
                .GetCurrentPatternAs::<IUIAutomationItemContainerPattern>(
                    UIA_ItemContainerPatternId,
                )
        }) else {
            continue;
        };
        unsafe {
            // 1. exact name
            let mut v = VARIANT::default();
            (*v.Anonymous.Anonymous).vt = VT_BSTR;
            (*v.Anonymous.Anonymous).Anonymous.bstrVal =
                std::mem::ManuallyDrop::new(windows::core::BSTR::from(orig));
            let exact =
                ic.FindItemByProperty(None::<&IUIAutomationElement>, UIA_NamePropertyId, &v);
            let _ = VariantClear(&mut v);
            if let Ok(e) = exact {
                add(&e, &mut out);
                return (out, 0, true);
            }
            // 2. bounded walk comparing names
            let empty = VARIANT::default();
            let mut prev: Option<IUIAutomationElement> = None;
            for _ in 0..DEEP_MAX_ITEMS {
                if Instant::now() > deadline || out.len() >= 20 {
                    break;
                }
                let Ok(e) = ic.FindItemByProperty(prev.as_ref(), UIA_PROPERTY_ID(0), &empty) else {
                    break;
                };
                scanned += 1;
                let name = e.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                if name.to_lowercase().contains(q) {
                    add(&e, &mut out);
                }
                prev = Some(e);
            }
        }
    }
    (out, scanned, false)
}

const MAX_TEXT: usize = 20_000;

fn element_text(el: &IUIAutomationElement) -> Option<String> {
    unsafe {
        if let Ok(tp) = el.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
            if let Ok(r) = tp.DocumentRange() {
                if let Ok(s) = r.GetText(200_000) {
                    return Some(s.to_string());
                }
            }
        }
        if let Ok(vp) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            if let Ok(s) = vp.CurrentValue() {
                return Some(s.to_string());
            }
        }
        el.CurrentName().ok().map(|b| b.to_string())
    }
}

/// Keeps the last `last` lines (if given) and caps the size.
pub fn shape_text(s: &str, last: Option<usize>) -> (String, bool) {
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    let s = match last {
        Some(n) => {
            let lines: Vec<&str> = s.lines().collect();
            lines[lines.len().saturating_sub(n)..].join("\n")
        }
        None => s,
    };
    if s.chars().count() <= MAX_TEXT {
        return (s, false);
    }
    let cut: String = if last.is_some() {
        let n = s.chars().count();
        s.chars().skip(n - MAX_TEXT).collect()
    } else {
        s.chars().take(MAX_TEXT).collect()
    };
    (cut, true)
}

fn read_text(p: &Value) -> Result<Value, ActError> {
    let last = match p.get("last_lines") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_u64()
                .filter(|n| (1..=10_000).contains(n))
                .ok_or_else(|| bad("'last_lines' must be 1..=10000"))? as usize,
        ),
    };
    let raw = if let Some(r) = p.get("ref").filter(|v| !v.is_null()) {
        let t = resolve(r)?;
        if t.password
            || unsafe { t.el.0.CurrentIsPassword() }
                .map(|b| b.as_bool())
                .unwrap_or(false)
        {
            return Ok(json!({"text": "***", "masked": true}));
        }
        element_text(&t.el.0).unwrap_or_default()
    } else {
        let h = target_window(p)?;
        main_text(&snapshot_settled(h, false)?.0)
    };
    let (text, truncated) = shape_text(&raw, last);
    Ok(json!({"text": text, "truncated": truncated}))
}

/// A window's main text: the largest visible element with a TextPattern (editors, documents,
/// terminals — Windows Terminal exposes its buffer on a non-Document element, flow F6), else
/// the largest document/edit with a value, else the visible texts.
fn main_text(nodes: &[Node]) -> String {
    let area = |n: &&Node| n.info.rect.2 as i64 * n.info.rect.3 as i64;
    let usable = |n: &&Node| !n.info.password && !n.info.offscreen && n.info.rect.2 > 0;
    let best = nodes
        .iter()
        .skip(1)
        .filter(usable)
        .filter(|n| n.info.text)
        .max_by_key(area)
        .or_else(|| {
            nodes
                .iter()
                .skip(1)
                .filter(usable)
                .filter(|n| n.info.value.is_some() && matches!(n.info.role, "Document" | "Edit"))
                .max_by_key(area)
        });
    match best {
        Some(n) => element_text(&n.el.0).unwrap_or_default(),
        None => nodes
            .iter()
            .filter(|n| {
                n.info.role == "Text" && !n.info.offscreen && !n.info.name.trim().is_empty()
            })
            .map(|n| n.info.name.trim().to_string())
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

// ---------------------------------------------------------------------------------------------
// wait_for (P2-M1, ADR 0022): conditions polled in the core, no agent round trips.

/// Visible text of a tree (names + values, never password values), lowercase.
fn visible_text(nodes: &[Node]) -> String {
    let mut s = String::new();
    for n in nodes {
        if n.info.offscreen {
            continue;
        }
        s.push_str(&n.info.name.to_lowercase());
        s.push('\n');
        if let Some(v) = n.info.value.as_ref().filter(|_| !n.info.password) {
            s.push_str(&v.to_lowercase());
            s.push('\n');
        }
    }
    s
}

fn fingerprint(nodes: &[Node]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for n in nodes {
        n.info.line().hash(&mut h);
        n.info.rect.hash(&mut h);
    }
    h.finish()
}

fn wait_for(p: &Value) -> Result<Value, ActError> {
    let t0 = Instant::now();
    let timeout = match p.get("timeout_ms") {
        None | Some(Value::Null) => Duration::from_secs(10),
        Some(v) => Duration::from_millis(
            v.as_u64()
                .filter(|n| (100..=120_000).contains(n))
                .ok_or_else(|| bad("'timeout_ms' must be 100..=120000"))?,
        ),
    };
    let lower = |k: &str| {
        p.get(k)
            .and_then(Value::as_str)
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
    };
    let title = lower("window_title");
    let text = lower("text_contains");
    let ref_gone = p.get("ref_gone").filter(|v| !v.is_null()).cloned();
    if let Some(r) = &ref_gone {
        parse_elem_ref(r).ok_or_else(|| bad("'ref_gone' must look like \"e12\""))?;
    }
    let window_gone = p
        .get("window_gone")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let idle = match p.get("idle_ms") {
        None | Some(Value::Null) => None,
        Some(v) => Some(Duration::from_millis(
            v.as_u64()
                .filter(|n| (50..=60_000).contains(n))
                .ok_or_else(|| bad("'idle_ms' must be 50..=60000"))?,
        )),
    };
    // P6-M7 (ADR 0051): `delay_ms` = a plain wait first; alone it is the whole call.
    let delay = match p.get("delay_ms") {
        None | Some(Value::Null) => None,
        Some(v) => Some(Duration::from_millis(
            v.as_u64()
                .filter(|n| *n <= 60_000)
                .ok_or_else(|| bad("'delay_ms' must be 0..=60000"))?,
        )),
    };
    let no_conditions =
        title.is_none() && text.is_none() && ref_gone.is_none() && !window_gone && idle.is_none();
    if no_conditions && delay.is_none() {
        return Err(bad(
            "give delay_ms (a plain wait) or at least one condition: window_title, text_contains, ref_gone, window_gone, idle_ms",
        ));
    }
    if let Some(d) = delay {
        while t0.elapsed() < d {
            crate::safety::check()?;
            sleep((d - t0.elapsed()).min(Duration::from_millis(100)));
        }
        if no_conditions {
            return Ok(json!({"met": ["delay"], "waited_ms": t0.elapsed().as_millis() as u64}));
        }
    }
    // With a delay, the conditions' own timeout starts after it.
    let t0 = Instant::now();
    // The window: given, or the foreground one for tree conditions (none needed for a title
    // search across windows or for ref_gone).
    let window: Option<HWND> = match p.get("window").filter(|v| !v.is_null()) {
        Some(v) => {
            // A ref whose window is gone (or whose handle another app now owns) meets
            // `window_gone` at once; text (P6-M1a) and live refs resolve as everywhere else.
            let gone = winmgr::parse_ref(v).is_some_and(|n| !winmgr::exists(n as HWND));
            if window_gone && gone {
                None
            } else {
                match target_window(p) {
                    Err(e) if window_gone && e.code == "STALE_REF" => None,
                    r => Some(r?),
                }
            }
        }
        None if text.is_some() || idle.is_some() => Some(target_window(p)?),
        None => None,
    };
    if window_gone && window.is_none() && p.get("window").is_none_or(Value::is_null) {
        return Err(bad("'window_gone' needs 'window'"));
    }
    let mut last_fp: Option<u64> = None;
    let mut last_change = Instant::now();
    loop {
        crate::safety::check()?;
        let mut unmet: Vec<&str> = Vec::new();
        let mut found: Option<HWND> = window;
        let alive = window.is_some_and(winmgr::exists);
        if window_gone && alive {
            unmet.push("window_gone");
        }
        if let Some(t) = &title {
            let hit = match window {
                Some(w) => alive && winmgr::title(w).to_lowercase().contains(t.as_str()),
                None => {
                    found = winmgr::listable_windows().into_iter().find(|&w| {
                        !crate::safety::is_own_window(w)
                            && winmgr::title(w).to_lowercase().contains(t.as_str())
                    });
                    found.is_some()
                }
            };
            if !hit {
                unmet.push("window_title");
            }
        }
        if let Some(r) = &ref_gone {
            let still = match resolve(r) {
                Ok(t) => unsafe { t.el.0.CurrentIsOffscreen() }
                    .map(|b| !b.as_bool())
                    .unwrap_or(false),
                Err(_) => false,
            };
            if still {
                unmet.push("ref_gone");
            }
        }
        if text.is_some() || idle.is_some() {
            match window.filter(|_| alive) {
                None => {
                    if text.is_some() {
                        unmet.push("text_contains");
                    }
                    if idle.is_some() {
                        unmet.push("idle_ms");
                    }
                }
                Some(w) => {
                    let nodes = snapshot(w, true)?;
                    if let Some(t) = &text {
                        let hit = visible_text(&nodes).contains(t.as_str())
                            || main_text(&nodes).to_lowercase().contains(t.as_str());
                        if !hit {
                            unmet.push("text_contains");
                        }
                    }
                    if let Some(d) = idle {
                        let fp = fingerprint(&nodes);
                        if last_fp != Some(fp) {
                            last_fp = Some(fp);
                            last_change = Instant::now();
                        }
                        if last_change.elapsed() < d {
                            unmet.push("idle_ms");
                        }
                    }
                }
            }
        }
        if unmet.is_empty() {
            let mut met = Vec::new();
            for (k, on) in [
                ("window_title", title.is_some()),
                ("text_contains", text.is_some()),
                ("ref_gone", ref_gone.is_some()),
                ("window_gone", window_gone),
                ("idle_ms", idle.is_some()),
            ] {
                if on {
                    met.push(k);
                }
            }
            return Ok(json!({
                "met": met,
                "window": found.filter(|&w| winmgr::exists(w)).map(winmgr::window_ref),
                "ms": t0.elapsed().as_millis() as u64,
            }));
        }
        if t0.elapsed() >= timeout {
            return Err(err(
                "TIMEOUT",
                format!(
                    "not met after {} ms: {}",
                    timeout.as_millis(),
                    unmet.join(", ")
                ),
                "Check the condition (observe the window), or wait longer with timeout_ms.",
            ));
        }
        sleep(Duration::from_millis(250));
    }
}

// ---------------------------------------------------------------------------------------------
// Background actions

/// If a background action made the target's process take the foreground (S10: Calculator
/// activates itself on Invoke), give it back to the window the user had.
fn keep_foreground<T>(
    target: HWND,
    f: impl FnOnce() -> Result<T, ActError>,
) -> Result<(T, bool), ActError> {
    let before = winmgr::foreground_root();
    let out = f()?;
    if before.is_null() || before == target {
        return Ok((out, false));
    }
    let pid = winmgr::pid_of(target);
    let stolen = move || {
        let now = winmgr::foreground_root();
        now != before && !now.is_null() && winmgr::pid_of(now) == pid && winmgr::exists(before)
    };
    let mut restored = false;
    for wait in [0u64, 40] {
        sleep(Duration::from_millis(wait));
        if stolen() {
            restored = winmgr::focus(before) || restored;
        }
    }
    // Some apps activate a little later (seen with Calculator). Keep watching briefly without
    // blocking the reply; stop as soon as any newer action starts (it may focus on purpose).
    let gen = input::action_generation();
    let before_addr = before as usize;
    std::thread::spawn(move || {
        let before = before_addr as HWND;
        for _ in 0..16 {
            sleep(Duration::from_millis(25));
            if input::action_generation() != gen || crate::safety::stopped() {
                return;
            }
            let now = winmgr::foreground_root();
            if now != before
                && !now.is_null()
                && winmgr::pid_of(now) == pid
                && winmgr::exists(before)
            {
                winmgr::focus(before);
            }
        }
    });
    Ok((out, restored))
}

/// The background ways to click one element, in order. Ok(None) = none fits.
unsafe fn click_patterns(
    el: &IUIAutomationElement,
    row: bool,
) -> Result<Option<&'static str>, ActError> {
    if row {
        if let Ok(p) =
            el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
        {
            p.Select().map_err(|e| com_err("select", e))?;
            return Ok(Some("select"));
        }
    }
    if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationInvokePattern>(UIA_InvokePatternId) {
        p.Invoke().map_err(|e| com_err("invoke", e))?;
        return Ok(Some("invoke"));
    }
    if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationTogglePattern>(UIA_TogglePatternId) {
        p.Toggle().map_err(|e| com_err("toggle", e))?;
        return Ok(Some("toggle"));
    }
    if let Ok(p) =
        el.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
    {
        p.Select().map_err(|e| com_err("select", e))?;
        return Ok(Some("select"));
    }
    if let Ok(p) =
        el.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    {
        let state = p.CurrentExpandCollapseState().map(|s| s.0).unwrap_or(0);
        if state == 3 {
            // leaf: nothing to do here, fall through to the next option
        } else if state == 0 {
            p.Expand().map_err(|e| com_err("expand", e))?;
            return Ok(Some("expand"));
        } else {
            p.Collapse().map_err(|e| com_err("collapse", e))?;
            return Ok(Some("collapse"));
        }
    }
    if let Ok(p) = el.GetCurrentPatternAs::<IUIAutomationLegacyIAccessiblePattern>(
        UIA_LegacyIAccessiblePatternId,
    ) {
        let action = p
            .CurrentDefaultAction()
            .map(|b| b.to_string())
            .unwrap_or_default();
        if !action.is_empty() {
            p.DoDefaultAction()
                .map_err(|e| com_err("default action", e))?;
            return Ok(Some("default_action"));
        }
    }
    Ok(None)
}

/// `click(ref)` in the background. Ok(None) = no pattern fits (caller uses real input).
pub fn click_background(t: &Target) -> Result<Option<Value>, ActError> {
    if !t.enabled() {
        return Err(err(
            "INVALID_ARGS",
            "the element is disabled",
            "Wait until it is enabled (observe shows (off)).",
        ));
    }
    // A click on a list/tree row selects it; Invoke on a row means "open" (a file in
    // Explorer, ADR 0035), which a single click never does.
    let ct = unsafe { t.el.0.CurrentControlType() }
        .map(|c| c.0)
        .unwrap_or(0);
    let row = matches!(
        ct,
        x if x == UIA_ListItemControlTypeId.0
            || x == UIA_DataItemControlTypeId.0
            || x == UIA_TreeItemControlTypeId.0
    );
    let res = keep_foreground(t.window, || unsafe {
        if let Some(how) = click_patterns(&t.el.0, row)? {
            return Ok(Some(how));
        }
        // P6-M6 (ADR 0049): a web page's clickable <div> shows up as its Text; the click
        // handler sits on the parent (a Group with Invoke). Try the direct parent once.
        if matches!(role_name(ct), "Text" | "Image") {
            let u = uia()?;
            if let Ok(parent) = u
                .a
                .ControlViewWalker()
                .and_then(|w| w.GetParentElement(&t.el.0))
            {
                let prole = role_name(parent.CurrentControlType().map(|c| c.0).unwrap_or(0));
                if matches!(prole, "Group" | "Custom" | "Pane" | "ListItem" | "DataItem") {
                    if let Some(how) = click_patterns(&parent, false)? {
                        return Ok(Some(match how {
                            "invoke" => "parent_invoke",
                            "default_action" => "parent_default_action",
                            "toggle" => "parent_toggle",
                            "select" => "parent_select",
                            _ => "parent",
                        }));
                    }
                }
            }
        }
        Ok(None)
    })?;
    Ok(res
        .0
        .map(|how| json!({"mode": "background", "via": how, "foreground_restored": res.1})))
}

/// `scroll(ref)`: ScrollItem into view, or Scroll on a container. Ok(None) = use real wheel.
pub fn scroll_background(t: &Target, dx: i32, dy: i32) -> Result<Option<Value>, ActError> {
    let res = keep_foreground(t.window, || unsafe {
        if dx == 0 && dy == 0 {
            if let Some(p) = t.pattern::<IUIAutomationScrollItemPattern>(UIA_ScrollItemPatternId) {
                p.ScrollIntoView()
                    .map_err(|e| com_err("scroll into view", e))?;
                return Ok(Some("scroll_into_view"));
            }
            return Ok(None);
        }
        if let Some(p) = t.pattern::<IUIAutomationScrollPattern>(UIA_ScrollPatternId) {
            let step = |n: i32| {
                if n > 0 {
                    ScrollAmount_SmallIncrement
                } else if n < 0 {
                    ScrollAmount_SmallDecrement
                } else {
                    ScrollAmount_NoAmount
                }
            };
            for i in 0..dx.abs().max(dy.abs()) {
                let h = if i < dx.abs() {
                    step(dx)
                } else {
                    ScrollAmount_NoAmount
                };
                let v = if i < dy.abs() {
                    step(dy)
                } else {
                    ScrollAmount_NoAmount
                };
                p.Scroll(h, v).map_err(|e| com_err("scroll", e))?;
            }
            return Ok(Some("scroll"));
        }
        Ok(None)
    })?;
    Ok(res
        .0
        .map(|how| json!({"mode": "background", "via": how, "foreground_restored": res.1})))
}

fn set_value(p: &Value) -> Result<Value, ActError> {
    let value = p
        .get("value")
        .and_then(Value::as_str)
        .ok_or_else(|| bad("'value' (string) is required"))?;
    if value.chars().count() > 10_000 {
        return Err(bad("'value' is limited to 10,000 characters"));
    }
    let t = resolve(p.get("ref").ok_or_else(|| bad("'ref' is required"))?)?;
    if t.password
        || unsafe { t.el.0.CurrentIsPassword() }
            .map(|b| b.as_bool())
            .unwrap_or(false)
    {
        return Err(err(
            "NEEDS_HUMAN",
            "this is a password field; the agent never enters passwords",
            "Ask the user to type it (ask_human).",
        ));
    }
    if !t.enabled() {
        return Err(err(
            "INVALID_ARGS",
            "the element is disabled",
            "Wait until it is enabled.",
        ));
    }
    let role = role_name(unsafe { t.el.0.CurrentControlType() }.map(|c| c.0).unwrap_or(0));
    // P6-M6 (ADR 0049): a drop-down (HTML <select>, desktop combo) takes its value by
    // selecting the option: writing the value text was "ok" but changed nothing on web pages.
    if role == "ComboBox" {
        if let Some(v) = set_combo(&t, value)? {
            return Ok(v);
        }
    }
    let (via, restored) = keep_foreground(t.window, || unsafe {
        if let Some(vp) = t.pattern::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            if vp.CurrentIsReadOnly().map(|b| b.as_bool()).unwrap_or(false) {
                return Err(err(
                    "INVALID_ARGS",
                    "the element is read-only",
                    "Pick an editable element (observe shows its value).",
                ));
            }
            vp.SetValue(&BSTR::from(value))
                .map_err(|e| com_err("set value", e))?;
            return Ok("value");
        }
        if let Some(lp) =
            t.pattern::<IUIAutomationLegacyIAccessiblePattern>(UIA_LegacyIAccessiblePatternId)
        {
            let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
            if lp.SetValue(windows::core::PCWSTR(wide.as_ptr())).is_ok() {
                return Ok("legacy_value");
            }
        }
        Err(err(
            "INVALID_ARGS",
            "the element has no settable value",
            "Use click(ref) then type instead.",
        ))
    })?;
    let mut out = json!({"mode": "background", "via": via, "chars": value.chars().count(), "foreground_restored": restored});
    // A browser's rich-text editor takes the text but its page code is not told (no input
    // event); UIA cannot tell it from a plain <input>, so say so for every browser field.
    if role == "Edit" && winmgr::is_chromium(t.window) {
        out["note"] = json!("in a browser, rich-text editors may not notice set_value; if the page does not react, use type {ref, text}");
    }
    Ok(out)
}

/// Which option matches `value`: exact name first, then trimmed and case-insensitive (pure).
pub fn match_option(options: &[String], value: &str) -> Option<usize> {
    options.iter().position(|o| o == value).or_else(|| {
        let v = value.trim().to_lowercase();
        options.iter().position(|o| o.trim().to_lowercase() == v)
    })
}

/// The option elements (ListItem descendants) of a drop-down, with their names.
fn combo_options(el: &IUIAutomationElement) -> Result<Vec<(String, IUIAutomationElement)>, ActError> {
    let u = uia()?;
    let mut out = Vec::new();
    unsafe {
        let all_cond = u
            .a
            .CreateTrueCondition()
            .map_err(|e| com_err("CreateTrueCondition", e))?;
        // Walk the raw view (FindAll missed Chromium's open <select> list: combo > List >
        // ListItem, measured 2026-11-10). Depth ≤ 3, ≤ 500 options.
        if let Ok(walker) = u.a.RawViewWalker() {
            let mut stack: Vec<(IUIAutomationElement, u32)> = Vec::new();
            if let Ok(c) = walker.GetFirstChildElement(el) {
                stack.push((c, 1));
            }
            while let Some((o, depth)) = stack.pop() {
                if out.len() >= 500 {
                    break;
                }
                if let Ok(next) = walker.GetNextSiblingElement(&o) {
                    stack.push((next, depth));
                }
                if o.CurrentControlType().map(|c| c.0).unwrap_or(0) == UIA_ListItemControlTypeId.0
                {
                    let name = o.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                    out.push((name, o));
                } else if depth < 3 {
                    if let Ok(c) = walker.GetFirstChildElement(&o) {
                        stack.push((c, depth + 1));
                    }
                }
            }
        }
        // Chromium's <select>: the options are not under the combo element; the selected
        // option is reachable through SelectionPattern, and its list holds all of them.
        if out.is_empty() {
            if let Ok(sp) =
                el.GetCurrentPatternAs::<IUIAutomationSelectionPattern>(UIA_SelectionPatternId)
            {
                if let Some(list) = sp
                    .GetCurrentSelection()
                    .ok()
                    .and_then(|s| s.GetElement(0).ok())
                    .and_then(|sel| {
                        u.a.RawViewWalker()
                            .ok()
                            .and_then(|w| w.GetParentElement(&sel).ok())
                    })
                {
                    if let Ok(kids) = list.FindAll(TreeScope_Children, &all_cond) {
                        for i in 0..kids.Length().unwrap_or(0).min(500) {
                            let Ok(o) = kids.GetElement(i) else { continue };
                            if o.CurrentControlType().map(|c| c.0).unwrap_or(0)
                                == UIA_ListItemControlTypeId.0
                            {
                                let name =
                                    o.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                                out.push((name, o));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(out)
}

/// The drop-down's shown value: ValuePattern, else the selected option's name.
fn combo_value(el: &IUIAutomationElement) -> String {
    unsafe {
        if let Ok(vp) = el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId) {
            if let Ok(v) = vp.CurrentValue() {
                return v.to_string();
            }
        }
        if let Ok(sp) = el.GetCurrentPatternAs::<IUIAutomationSelectionPattern>(UIA_SelectionPatternId) {
            if let Ok(sel) = sp.GetCurrentSelection() {
                if let Ok(first) = sel.GetElement(0) {
                    return first.CurrentName().map(|b| b.to_string()).unwrap_or_default();
                }
            }
        }
    }
    String::new()
}

/// set_value on a ComboBox: select the matching option (expanding the list if needed), then
/// check the shown value. Ok(None): an editable combo whose text is not an option (the caller
/// writes the text instead).
fn set_combo(t: &Target, value: &str) -> Result<Option<Value>, ActError> {
    let el = &t.el.0;
    let ec = unsafe {
        el.GetCurrentPatternAs::<IUIAutomationExpandCollapsePattern>(UIA_ExpandCollapsePatternId)
    }
    .ok();
    // Desktop combos list their options while closed; Chromium's <select> only while open,
    // and opening it activates the browser (its popup takes the keyboard). So: wait until
    // the user is idle (a key pressed now would land in the list), then open, select, close
    // and read back inside ONE foreground hold — giving the foreground back earlier closes
    // the popup and the options vanish (measured 2026-11-10).
    let need_open = combo_options(el)?.is_empty() && ec.is_some();
    if need_open {
        crate::input::wait_user_idle(&crate::input::Polite {
            idle_ms: 1_500,
            timeout_ms: 30_000,
            restore_cursor: false,
        })?;
    }
    enum Done {
        Selected(String, String),
        NoMatch(Vec<String>),
    }
    let (done, _) = keep_foreground(t.window, || unsafe {
        let mut opened = false;
        let mut opts = combo_options(el)?;
        if opts.is_empty() {
            if let Some(p) = &ec {
                p.Expand().map_err(|e| com_err("expand", e))?;
                opened = true;
                let t0 = Instant::now();
                while opts.is_empty() && t0.elapsed() < Duration::from_millis(1500) {
                    sleep(Duration::from_millis(100));
                    opts = combo_options(el)?;
                }
            }
        }
        let close = |opened: bool| {
            if opened {
                if let Some(p) = &ec {
                    if p.CurrentExpandCollapseState().map(|s| s.0).unwrap_or(0) != 0 {
                        let _ = p.Collapse();
                    }
                }
            }
        };
        let names: Vec<String> = opts.iter().map(|(n, _)| n.clone()).collect();
        let Some(i) = match_option(&names, value) else {
            close(opened);
            return Ok(Done::NoMatch(names));
        };
        let pick = |o: &IUIAutomationElement| -> Result<(), ActError> {
            o.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId)
                .map_err(|e| com_err("the option cannot be selected", e))?
                .Select()
                .map_err(|e| com_err("select", e))
        };
        let name = names[i].clone();
        pick(&opts[i].1)?;
        sleep(Duration::from_millis(150));
        // Some lists only commit a selection made while they are open.
        if combo_value(el) != name && !opened {
            if let Some(p) = &ec {
                p.Expand().map_err(|e| com_err("expand", e))?;
                opened = true;
                sleep(Duration::from_millis(300));
                if let Some((_, o)) = combo_options(el)?.iter().find(|(n, _)| *n == name) {
                    pick(o)?;
                    sleep(Duration::from_millis(150));
                }
            }
        }
        close(opened);
        sleep(Duration::from_millis(100));
        Ok(Done::Selected(name, combo_value(el)))
    })?;
    let names = match done {
        Done::Selected(name, shown) => {
            if shown != name {
                return Err(err(
                    "NEEDS_HUMAN",
                    format!(
                        "selected \"{}\" but the drop-down still shows \"{}\"",
                        clip(&name, 60),
                        clip(&shown, 60)
                    ),
                    "Try click(ref) on the drop-down, then click the option; else ask the user.",
                ));
            }
            return Ok(Some(
                json!({"mode": "background", "via": "select_option", "option": name}),
            ));
        }
        Done::NoMatch(names) => names,
    };
    {
        let editable = unsafe {
            let u = uia()?;
            let cond = u.a.CreateTrueCondition().map_err(|e| com_err("cond", e))?;
            el.FindAll(TreeScope_Children, &cond)
                .ok()
                .map(|kids| {
                    (0..kids.Length().unwrap_or(0)).any(|k| {
                        kids.GetElement(k)
                            .and_then(|c| c.CurrentControlType())
                            .map(|c| c.0 == UIA_EditControlTypeId.0)
                            .unwrap_or(false)
                    })
                })
                .unwrap_or(false)
        };
        if editable || names.is_empty() {
            return Ok(None);
        }
        let list: Vec<String> = names.iter().take(20).map(|n| format!("\"{}\"", clip(n, 40))).collect();
        return Err(err(
            "ELEMENT_NOT_FOUND",
            format!(
                "no option \"{}\" in this drop-down; options: {}{}",
                clip(value, 60),
                list.join(", "),
                if names.len() > 20 { ", …" } else { "" }
            ),
            "Pass one of the option names exactly.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(role: &'static str, name: &str) -> Info {
        Info {
            role,
            name: name.into(),
            rect: (0, 0, 10, 10),
            enabled: true,
            ..Default::default()
        }
    }

    #[test]
    fn lines_are_compact_and_mask_passwords() {
        assert_eq!(info("Button", "Seven").line(), "Button \"Seven\"");
        let mut e = info("Edit", "Search");
        e.value = Some("abc".into());
        assert_eq!(e.line(), "Edit \"Search\" = \"abc\"");
        e.password = true;
        e.value = Some(String::new());
        assert_eq!(e.line(), "Edit \"Search\" = ***");
        let mut c = info("CheckBox", "Wrap");
        c.toggle = Some(1);
        c.enabled = false;
        assert_eq!(c.line(), "CheckBox \"Wrap\" [x] (off)");
        let mut u = info("Edit", "");
        u.aid = "searchBox".into();
        assert_eq!(u.line(), "Edit #searchBox");
    }

    // P6-M6 (ADR 0049): set_value on a drop-down picks the option by name.
    #[test]
    fn options_match_exactly_then_loosely() {
        let o: Vec<String> = ["India", "Japan", " Peru ", "japan"].iter().map(|s| s.to_string()).collect();
        assert_eq!(match_option(&o, "japan"), Some(3), "exact wins");
        assert_eq!(match_option(&o, "JAPAN"), Some(1), "then case-insensitive");
        assert_eq!(match_option(&o, "peru"), Some(2), "trimmed");
        assert_eq!(match_option(&o, "Chile"), None);
    }

    #[test]
    fn a_missing_chromium_page_says_why() {
        assert_eq!(page_state(true, false, true), Some("minimized"));
        assert_eq!(page_state(true, false, false), Some("not_exposed"));
        assert_eq!(page_state(true, true, true), None);
        assert_eq!(page_state(false, false, true), None); // not Chromium: no claim
        assert!(page_note("minimized").contains("reveal:true"));
        assert!(page_note("not_exposed").contains("focus_window"));
    }

    #[test]
    fn dialog_lines_name_the_buttons_first() {
        let b = vec![("e46".to_string(), "OK".to_string()), ("e47".to_string(), "Cancel".to_string())];
        assert_eq!(
            dialog_line("e45", "This page says", "Saved", &b, true, false),
            "!! dialog e45 \"This page says\" (blocking): \"Saved\" — buttons: e46 \"OK\", e47 \"Cancel\". Answer it first; the window may not respond until it closes."
        );
        let none = dialog_line("w9", "Find", "", &[], false, false);
        assert!(none.starts_with("!! dialog w9 \"Find\" — no buttons found"), "{none}");
        let adm = dialog_line("w9", "Setup", "x", &b, true, true);
        assert!(adm.contains("elevated_helper") && !adm.contains("e46"), "{adm}");
        let v = dialog_value("e45", "T", "m", &b, "in_page", true);
        assert_eq!(v["buttons"][1]["ref"], "e47");
        assert_eq!(v["kind"], "in_page");
    }

    #[test]
    fn clip_escapes_and_cuts() {
        assert_eq!(clip("a\"b\nc", 10), "a\\\"b c");
        assert_eq!(clip("abcdefghij", 5), "abcd…");
    }

    #[test]
    fn ranking_keeps_interactive_first() {
        assert_eq!(info("Button", "OK").rank(), Some(0));
        assert_eq!(info("Edit", "").rank(), Some(1));
        assert_eq!(info("Text", "Hello").rank(), Some(2));
        assert_eq!(info("Pane", "").rank(), None);
        let mut off = info("Button", "Hidden");
        off.offscreen = true;
        assert_eq!(off.rank(), None);
        let mut custom = info("Pane", "Clickable");
        custom.invoke = true;
        assert_eq!(custom.rank(), Some(0), "custom control with Invoke is kept");
    }

    #[test]
    fn select_respects_cap_and_order() {
        let ranks = vec![Some(2), Some(0), None, Some(1), Some(0)];
        assert_eq!(select(&ranks, 10), (vec![0, 1, 3, 4], 0));
        assert_eq!(select(&ranks, 2), (vec![1, 4], 2));
    }

    #[test]
    fn diff_reports_changes() {
        let old: BTreeMap<u32, String> =
            [(1, "Button \"A\"".into()), (2, "Text \"0\"".into())].into();
        let new: BTreeMap<u32, String> = [
            (1, "Button \"A\"".into()),
            (2, "Text \"7\"".into()),
            (3, "Button \"B\"".into()),
        ]
        .into();
        let (lines, c, a, r) = diff(&old, &new);
        assert_eq!((c, a, r), (1, 1, 0));
        assert_eq!(lines, vec!["~ e2 Text \"7\"", "+ e3 Button \"B\""]);
    }

    #[test]
    fn text_shaping() {
        assert_eq!(
            shape_text("a\r\nb\nc", Some(2)),
            ("b\nc".to_string(), false)
        );
        let long = "x".repeat(MAX_TEXT + 5);
        let (t, cut) = shape_text(&long, None);
        assert!(cut && t.chars().count() == MAX_TEXT);
    }

    #[test]
    fn refs_parse() {
        assert_eq!(parse_elem_ref(&json!("e12")), Some(12));
        assert_eq!(parse_elem_ref(&json!("w12")), None);
        assert_eq!(parse_elem_ref(&json!(12)), None);
        assert_eq!(parse_elem_ref(&json!("e0")), None);
    }

    #[test]
    fn role_names() {
        assert_eq!(role_name(50000), "Button");
        assert_eq!(role_name(50033), "Pane");
        assert_eq!(role_name(50040), "AppBar");
        assert_eq!(role_name(1), "Element");
    }
}
