//! Browser bridge client (P2-M6, ADR 0032): the core talks to the XtraDevPilot bridge
//! (`ws://127.0.0.1:42819`, pairing token) so web pages can be driven by CSS selector or text,
//! while every action still passes the core's policy, dialogs, kill switch and action log.
//!
//! Security: loopback only, no Origin header, `Sec-WebSocket-Accept` checked, the token goes
//! in an `AUTH` message (never in the URL), and only an allowlist of actions is exposed (no
//! script execution, cookies, storage, network mocks, HAR, source download, file upload).
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Map, Value};

use crate::input::ActError;
use crate::policy::Facts;

const DEFAULT_PORT: u16 = 42819;
/// Longest text returned to the agent (DOM / AX tree output).
pub const MAX_TEXT: usize = 60_000;

// =============================================================================================
// Minimal RFC 6455 codec (client side; the frame reader also unmasks for tests' fake server)

pub mod ws {
    use std::io::{self, Read};

    /// Largest message accepted (screenshots/DOM from the bridge stay well below).
    pub const MAX_MESSAGE: usize = 64 * 1024 * 1024;
    pub const OP_CONT: u8 = 0x0;
    pub const OP_TEXT: u8 = 0x1;
    pub const OP_BINARY: u8 = 0x2;
    pub const OP_CLOSE: u8 = 0x8;
    pub const OP_PING: u8 = 0x9;
    pub const OP_PONG: u8 = 0xA;

    pub fn sha1(data: &[u8]) -> [u8; 20] {
        let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
        let mut msg = data.to_vec();
        let bits = (data.len() as u64).wrapping_mul(8);
        msg.push(0x80);
        while msg.len() % 64 != 56 {
            msg.push(0);
        }
        msg.extend_from_slice(&bits.to_be_bytes());
        for chunk in msg.chunks(64) {
            let mut w = [0u32; 80];
            for i in 0..16 {
                w[i] = u32::from_be_bytes([
                    chunk[4 * i],
                    chunk[4 * i + 1],
                    chunk[4 * i + 2],
                    chunk[4 * i + 3],
                ]);
            }
            for i in 16..80 {
                w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
            }
            let [mut a, mut b, mut c, mut d, mut e] = h;
            for (i, wi) in w.iter().enumerate() {
                let (f, k) = match i {
                    0..=19 => ((b & c) | (!b & d), 0x5A827999),
                    20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                    40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                    _ => (b ^ c ^ d, 0xCA62C1D6),
                };
                let t = a
                    .rotate_left(5)
                    .wrapping_add(f)
                    .wrapping_add(e)
                    .wrapping_add(k)
                    .wrapping_add(*wi);
                e = d;
                d = c;
                c = b.rotate_left(30);
                b = a;
                a = t;
            }
            for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
                *x = x.wrapping_add(y);
            }
        }
        let mut out = [0u8; 20];
        for (i, v) in h.iter().enumerate() {
            out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
        }
        out
    }

    pub fn base64(data: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut s = String::new();
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

    /// `Sec-WebSocket-Accept` for a `Sec-WebSocket-Key`.
    pub fn accept_key(key: &str) -> String {
        base64(&sha1(
            format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes(),
        ))
    }

    /// One frame; `mask` is required for client → server frames.
    pub fn encode_frame(opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
        let mut f = vec![0x80 | (opcode & 0x0f)];
        let m = if mask.is_some() { 0x80 } else { 0 };
        let n = payload.len();
        if n < 126 {
            f.push(m | n as u8);
        } else if n <= 0xffff {
            f.push(m | 126);
            f.extend_from_slice(&(n as u16).to_be_bytes());
        } else {
            f.push(m | 127);
            f.extend_from_slice(&(n as u64).to_be_bytes());
        }
        match mask {
            Some(k) => {
                f.extend_from_slice(&k);
                f.extend(payload.iter().enumerate().map(|(i, b)| b ^ k[i % 4]));
            }
            None => f.extend_from_slice(payload),
        }
        f
    }

    /// Reads one frame: (fin, opcode, payload), unmasked.
    pub fn read_frame(r: &mut impl Read) -> io::Result<(bool, u8, Vec<u8>)> {
        let mut h = [0u8; 2];
        r.read_exact(&mut h)?;
        let fin = h[0] & 0x80 != 0;
        let op = h[0] & 0x0f;
        let masked = h[1] & 0x80 != 0;
        let mut len = (h[1] & 0x7f) as u64;
        if len == 126 {
            let mut b = [0u8; 2];
            r.read_exact(&mut b)?;
            len = u16::from_be_bytes(b) as u64;
        } else if len == 127 {
            let mut b = [0u8; 8];
            r.read_exact(&mut b)?;
            len = u64::from_be_bytes(b);
        }
        if len > MAX_MESSAGE as u64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "frame too large",
            ));
        }
        let mut key = [0u8; 4];
        if masked {
            r.read_exact(&mut key)?;
        }
        let mut p = vec![0u8; len as usize];
        r.read_exact(&mut p)?;
        if masked {
            for (i, b) in p.iter_mut().enumerate() {
                *b ^= key[i % 4];
            }
        }
        Ok((fin, op, p))
    }

    /// Reads one whole message (joins fragments). Pings are handed to `on_ping` (to answer
    /// with a pong); Ok(None) on a close frame.
    pub fn read_message(
        r: &mut impl Read,
        on_ping: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<Option<(u8, Vec<u8>)>> {
        let mut data: Vec<u8> = Vec::new();
        let mut first_op: Option<u8> = None;
        loop {
            let (fin, op, p) = read_frame(r)?;
            match op {
                OP_PING => on_ping(&p)?,
                OP_PONG => {}
                OP_CLOSE => return Ok(None),
                OP_TEXT | OP_BINARY | OP_CONT => {
                    if op != OP_CONT {
                        first_op = Some(op);
                    }
                    if data.len() + p.len() > MAX_MESSAGE {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "message too large",
                        ));
                    }
                    data.extend_from_slice(&p);
                    if fin {
                        return Ok(Some((first_op.unwrap_or(OP_TEXT), data)));
                    }
                }
                _ => return Err(io::Error::new(io::ErrorKind::InvalidData, "bad opcode")),
            }
        }
    }
}

// =============================================================================================
// Connection

struct Conn {
    s: TcpStream,
    next: u64,
}

static CONN: Mutex<Option<Conn>> = Mutex::new(None);

fn err(code: &'static str, message: impl Into<String>, hint: &'static str) -> ActError {
    ActError {
        code,
        message: message.into(),
        hint,
    }
}

fn bad(m: impl Into<String>) -> ActError {
    err(
        "INVALID_ARGS",
        m,
        "See the browser tool in docs/04-mcp-tool-contract.md.",
    )
}

const BRIDGE_HINT: &str = "The XtraDevPilot bridge runs with the XtraDevPilot MCP server; the XtraDevPilot extension must be loaded in Chrome. Ask the user to start them.";

fn port() -> u16 {
    std::env::var("XTRAOS_BROWSER_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn token() -> Option<String> {
    if let Ok(t) = std::env::var("XTRADEVPILOT_TOKEN") {
        if !t.trim().is_empty() {
            return Some(t.trim().to_string());
        }
    }
    let home = std::env::var("USERPROFILE").ok()?;
    let t = std::fs::read_to_string(
        std::path::Path::new(&home)
            .join(".xtradevpilot")
            .join("bridge-token"),
    )
    .ok()?;
    let t = t.trim().to_string();
    (t.len() >= 32).then_some(t)
}

fn mask_key() -> [u8; 4] {
    let b = crate::winsec::random_bytes(4).unwrap_or_else(|_| vec![0x5a, 0xa5, 0x3c, 0xc3]);
    [b[0], b[1], b[2], b[3]]
}

fn send_text(s: &mut TcpStream, text: &str) -> io::Result<()> {
    s.write_all(&ws::encode_frame(
        ws::OP_TEXT,
        text.as_bytes(),
        Some(mask_key()),
    ))?;
    s.flush()
}

fn recv_json(s: &mut TcpStream) -> io::Result<Value> {
    let mut w = s.try_clone()?;
    loop {
        let mut pong = |p: &[u8]| -> io::Result<()> {
            w.write_all(&ws::encode_frame(ws::OP_PONG, p, Some(mask_key())))
        };
        match ws::read_message(s, &mut pong)? {
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::ConnectionAborted,
                    "closed by the bridge",
                ))
            }
            Some((ws::OP_TEXT, d)) => {
                if let Ok(v) = serde_json::from_slice::<Value>(&d) {
                    return Ok(v);
                }
            }
            Some(_) => {}
        }
    }
}

fn connect() -> Result<Conn, ActError> {
    let port = port();
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port)); // loopback only, by construction
    let mut s = TcpStream::connect_timeout(&addr, Duration::from_secs(2)).map_err(|_| {
        err(
            "NEEDS_HUMAN",
            format!("the XtraDevPilot browser bridge is not running on 127.0.0.1:{port}"),
            BRIDGE_HINT,
        )
    })?;
    let _ = s.set_nodelay(true);
    s.set_read_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| err("INTERNAL", e.to_string(), ""))?;
    let tok = token().ok_or_else(|| {
        err(
            "NEEDS_HUMAN",
            "no XtraDevPilot pairing token (%USERPROFILE%\\.xtradevpilot\\bridge-token)",
            BRIDGE_HINT,
        )
    })?;
    let key = ws::base64(
        &crate::winsec::random_bytes(16).map_err(|e| err("INTERNAL", e.to_string(), ""))?,
    );
    let req = format!(
        "GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: xtraos-core\r\n\r\n"
    );
    let io_err = |e: io::Error| {
        err(
            "NEEDS_HUMAN",
            format!("browser bridge handshake failed: {e}"),
            BRIDGE_HINT,
        )
    };
    s.write_all(req.as_bytes()).map_err(io_err)?;
    // Response headers, byte by byte so no frame bytes are consumed.
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        s.read_exact(&mut b).map_err(io_err)?;
        head.push(b[0]);
        if head.len() > 8192 {
            return Err(err("INTERNAL", "browser bridge sent oversized headers", ""));
        }
    }
    let head = String::from_utf8_lossy(&head).to_string();
    let accept = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("sec-websocket-accept")
                .then(|| v.trim().to_string())
        })
        .unwrap_or_default();
    if !head.starts_with("HTTP/1.1 101") || accept != ws::accept_key(&key) {
        return Err(err(
            "NEEDS_HUMAN",
            "port answered, but not as the XtraDevPilot bridge (bad WebSocket handshake)",
            BRIDGE_HINT,
        ));
    }
    send_text(&mut s, &json!({"type": "AUTH", "token": tok}).to_string()).map_err(io_err)?;
    let r = recv_json(&mut s).map_err(io_err)?;
    if r["type"] != "AUTH_RESULT" || r["ok"] != true {
        return Err(err(
            "NEEDS_HUMAN",
            "the XtraDevPilot bridge refused the pairing token",
            "The token file and the running bridge disagree; restart the XtraDevPilot MCP server.",
        ));
    }
    Ok(Conn { s, next: 0 })
}

/// Drops the bridge connection (tests; the next call reconnects).
pub fn disconnect() {
    *CONN.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Sends one command and waits for its reply.
fn request(wire: &str, payload: Value, timeout: Duration) -> Result<Value, ActError> {
    let mut g = CONN.lock().unwrap_or_else(|e| e.into_inner());
    for attempt in 0..2 {
        if g.is_none() {
            *g = Some(connect()?);
        }
        let c = g.as_mut().expect("connected");
        c.next += 1;
        let id = c.next;
        let mut msg = match payload.clone() {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        msg.insert("action".into(), json!(wire));
        msg.insert("id".into(), json!(id));
        if send_text(&mut c.s, &Value::Object(msg).to_string()).is_err() {
            // Not sent: a stale connection. Reconnect once.
            *g = None;
            if attempt == 0 {
                continue;
            }
            return Err(err("NEEDS_HUMAN", "lost the browser bridge", BRIDGE_HINT));
        }
        let _ = c.s.set_read_timeout(Some(timeout));
        loop {
            match recv_json(&mut c.s) {
                Ok(v) if v["id"] == id => {
                    if let Some(r) = v.get("result") {
                        return Ok(r.clone());
                    }
                    return Err(map_error(v["error"].as_str().unwrap_or("bridge error")));
                }
                Ok(_) => continue, // not ours (e.g. late reply)
                Err(e) => {
                    *g = None;
                    return Err(
                        if matches!(
                            e.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                        ) {
                            err("TIMEOUT", format!("no reply from the browser within {} s: the action may or may not have run", timeout.as_secs()), "Check with tab_info / dom before repeating it.")
                        } else {
                            err(
                                "NEEDS_HUMAN",
                                format!("browser bridge connection lost: {e}"),
                                BRIDGE_HINT,
                            )
                        },
                    );
                }
            }
        }
    }
    Err(err(
        "NEEDS_HUMAN",
        "browser bridge unavailable",
        BRIDGE_HINT,
    ))
}

fn map_error(m: &str) -> ActError {
    let l = m.to_lowercase();
    if l.starts_with("unauthorized") || l.contains("no chrome browser is currently connected") {
        err("NEEDS_HUMAN", m.to_string(), BRIDGE_HINT)
    } else if l.contains("timeout") || l.contains("timed out") {
        err("TIMEOUT", m.to_string(), "")
    } else if l.contains("not found") || l.contains("no element") || l.contains("no match") {
        err(
            "ELEMENT_NOT_FOUND",
            m.to_string(),
            "Look at the page with dom or ax_tree first.",
        )
    } else {
        err("INTERNAL", format!("browser: {m}"), "")
    }
}

// =============================================================================================
// Tool: browser {action, …}

const OBSERVE_ACTIONS: &[&str] = &[
    "list_browsers",
    "list_tabs",
    "tab_info",
    "dom",
    "ax_tree",
    "wait_for",
];
pub const ACTIONS: &[&str] = &[
    "list_browsers",
    "list_tabs",
    "tab_info",
    "dom",
    "ax_tree",
    "wait_for",
    "click",
    "type",
    "press_key",
    "scroll",
    "navigate",
    "open_tab",
    "switch_browser",
];

pub fn is_observe(action: &str) -> bool {
    OBSERVE_ACTIONS.contains(&action)
}

fn s<'a>(p: &'a Value, k: &str) -> Option<&'a str> {
    p.get(k).and_then(Value::as_str)
}

fn need<'a>(p: &'a Value, k: &str) -> Result<&'a str, ActError> {
    s(p, k)
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| bad(format!("'{k}' is required")))
}

fn web_url(u: &str) -> Result<String, ActError> {
    let l = u.trim().to_lowercase();
    if l.starts_with("https://") || l.starts_with("http://") {
        Ok(u.trim().to_string())
    } else {
        Err(bad(
            "only http:// and https:// URLs (no javascript:, file:, data: or browser pages)",
        ))
    }
}

/// Validates the arguments and maps them to the bridge wire format: (action, payload, timeout).
pub fn plan(p: &Value) -> Result<(&'static str, Value, Duration), ActError> {
    let action = need(p, "action")?;
    let mut w = Map::new();
    if let Some(t) = p.get("tab_id").and_then(Value::as_i64) {
        w.insert("tabId".into(), json!(t));
    }
    if let Some(b) = p.get("browser_id").and_then(Value::as_i64) {
        w.insert("browserId".into(), json!(b));
    }
    let mut timeout = Duration::from_secs(20);
    let wire = match action {
        "list_browsers" => "LIST_BROWSERS",
        "list_tabs" => "LIST_TABS",
        "tab_info" => "GET_TAB_INFO",
        "dom" => {
            if let Some(r) = s(p, "root") {
                w.insert("rootSelector".into(), json!(r));
            }
            "GET_CLEAN_DOM"
        }
        "ax_tree" => "GET_AX_TREE",
        "wait_for" => {
            w.insert("selector".into(), json!(need(p, "selector")?));
            let ms = p.get("timeout_ms").and_then(Value::as_u64).unwrap_or(10_000).min(55_000);
            w.insert("timeoutMs".into(), json!(ms));
            if let Some(st) = s(p, "state") {
                if !["visible", "hidden", "attached", "detached"].contains(&st) {
                    return Err(bad("state: visible | hidden | attached | detached"));
                }
                w.insert("state".into(), json!(st));
            }
            timeout = Duration::from_millis(ms + 5_000);
            "WAIT_FOR_ELEMENT"
        }
        "click" => {
            let sel = match (s(p, "selector"), s(p, "text")) {
                (Some(sel), None) if !sel.trim().is_empty() => sel.to_string(),
                (None, Some(t)) if !t.trim().is_empty() => {
                    let contains = p.get("contains").and_then(Value::as_bool).unwrap_or(false);
                    format!("{}{t}", if contains { "text*=" } else { "text=" })
                }
                _ => return Err(bad("click needs exactly one of 'selector' or 'text'")),
            };
            w.insert("selector".into(), json!(sel));
            "CLICK_ELEMENT"
        }
        "type" => {
            w.insert("selector".into(), json!(need(p, "selector")?));
            let t = s(p, "text").ok_or_else(|| bad("'text' is required"))?;
            if t.chars().count() > 20_000 {
                return Err(bad("text is limited to 20,000 characters"));
            }
            w.insert("text".into(), json!(t));
            if let Some(a) = p.get("append").and_then(Value::as_bool) {
                w.insert("append".into(), json!(a));
            }
            timeout = Duration::from_secs(30);
            "TYPE_TEXT"
        }
        "press_key" => {
            w.insert("key".into(), json!(need(p, "key")?));
            if let Some(sel) = s(p, "selector") {
                w.insert("selector".into(), json!(sel));
            }
            for (a, b) in [("ctrl", "ctrlKey"), ("shift", "shiftKey"), ("alt", "altKey")] {
                if p.get(a).and_then(Value::as_bool) == Some(true) {
                    w.insert(b.into(), json!(true));
                }
            }
            "PRESS_KEY"
        }
        "scroll" => {
            if let Some(d) = s(p, "direction") {
                if !["up", "down", "left", "right", "top", "bottom"].contains(&d) {
                    return Err(bad("direction: up | down | left | right | top | bottom"));
                }
                w.insert("direction".into(), json!(d));
            }
            if let Some(a) = p.get("amount").and_then(Value::as_i64) {
                w.insert("amount".into(), json!(a));
            }
            if let Some(sel) = s(p, "to_selector") {
                w.insert("scrollToSelector".into(), json!(sel));
            }
            "SCROLL_PAGE"
        }
        "navigate" => {
            w.insert("url".into(), json!(web_url(need(p, "url")?)?));
            "NAVIGATE"
        }
        "open_tab" => {
            w.insert("url".into(), json!(web_url(need(p, "url")?)?));
            if let Some(b) = p.get("background").and_then(Value::as_bool) {
                w.insert("background".into(), json!(b));
            }
            "OPEN_TAB"
        }
        "switch_browser" => {
            if !w.contains_key("browserId") {
                return Err(bad("'browser_id' is required"));
            }
            "SWITCH_BROWSER"
        }
        "execute_script" | "manage_cookies" | "manage_storage" | "get_storage" | "upload_file"
        | "drop_files" | "export_har" | "download_sources" | "mock_network_response" => {
            return Err(crate::safety::blocked(format!(
                "browser '{action}' is not available through XtraOS (scripts, cookies, storage and file transfer stay with the user)"
            )))
        }
        o => {
            return Err(bad(format!(
                "unknown browser action '{o}' ({})",
                ACTIONS.join(", ")
            )))
        }
    };
    Ok((wire, Value::Object(w), timeout))
}

/// One line for logs and approval dialogs.
pub fn describe(p: &Value) -> String {
    let a = s(p, "action").unwrap_or("?");
    let detail = match a {
        "click" => s(p, "text")
            .map(|t| format!(" \"{}\"", crate::perceive::clip(t, 40)))
            .or_else(|| s(p, "selector").map(|t| format!(" {}", crate::perceive::clip(t, 60))))
            .unwrap_or_default(),
        "type" => format!(
            " {} characters into {}",
            s(p, "text").map(|t| t.chars().count()).unwrap_or(0),
            crate::perceive::clip(s(p, "selector").unwrap_or("?"), 60)
        ),
        "press_key" => format!(" {}", s(p, "key").unwrap_or("?")),
        "navigate" | "open_tab" => {
            format!(" {}", crate::perceive::clip(s(p, "url").unwrap_or("?"), 80))
        }
        _ => String::new(),
    };
    format!("browser {a}{detail}")
}

fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_lowercase()
}

/// Something that names a password / OTP / card field.
pub fn sensitive_selector(sel: &str) -> bool {
    let l = sel.to_lowercase();
    [
        "password",
        "passwd",
        "pwd",
        "otp",
        "one-time",
        "cvv",
        "cvc",
        "card-number",
        "cardnumber",
        "cc-number",
        "security code",
        "pincode",
        "pin-code",
    ]
    .iter()
    .any(|w| l.contains(w))
}

/// The element name the policy word rules look at: the text of a text click, the last quoted
/// value in a selector (aria-label='Send'), or the selector itself.
pub fn element_name(p: &Value) -> Option<String> {
    if let Some(t) = s(p, "text") {
        return Some(t.to_string());
    }
    let sel = s(p, "selector")?;
    let quoted: Vec<&str> = sel.split(['\'', '"']).collect();
    if quoted.len() >= 3 {
        return Some(quoted[quoted.len() - 2].to_string());
    }
    Some(sel.to_string())
}

/// Adds browser facts to a policy check (ADR 0032): for input actions, the target tab's host
/// and title (GET_TAB_INFO, 3 s) feed the existing messaging / purchase / delete / credential
/// rules.
pub fn add_facts(p: &Value, f: &mut Facts, what: &mut String) {
    let action = s(p, "action").unwrap_or("");
    f.method = format!("browser:{action}");
    *what = describe(p);
    if is_observe(action) || !ACTIONS.contains(&action) {
        return;
    }
    match action {
        "click" => f.element = element_name(p),
        "type" => {
            let sel = s(p, "selector").unwrap_or("");
            f.password = sensitive_selector(sel);
            f.enter = s(p, "text").is_some_and(|t| t.contains('\n'));
        }
        "press_key" => {
            let k = s(p, "key").unwrap_or("").to_lowercase();
            f.enter = k == "enter" || k == "return";
            f.password = s(p, "selector").is_some_and(sensitive_selector);
        }
        _ => {}
    }
    let mut info = json!({});
    if let Some(t) = p.get("tab_id") {
        info["tab_id"] = t.clone();
    }
    let mut q = info.clone();
    q["action"] = json!("tab_info");
    if let Ok((wire, payload, _)) = plan(&q) {
        if let Ok(tab) = request(wire, payload, Duration::from_secs(3)) {
            let host = host_of(tab["url"].as_str().unwrap_or(""));
            let title = tab["title"].as_str().unwrap_or("").to_lowercase();
            if !host.is_empty() {
                *what = format!("{what} on {host}");
            }
            // A browser "window" for the rules: process = chrome.exe, title = tab title + host.
            f.windows
                .push(("chrome.exe".into(), format!("{title} {host}")));
            if action == "click"
                && crate::policy::is_messaging("chrome.exe", &format!("{title} {host}"))
                && s(p, "selector").is_some_and(|x| x.to_lowercase().contains("send"))
            {
                f.element = Some("send".into());
            }
        }
    }
}

fn cut(t: String) -> String {
    if t.chars().count() <= MAX_TEXT {
        return t;
    }
    let mut c: String = t.chars().take(MAX_TEXT).collect();
    c.push_str(&format!(
        "\n… cut at {MAX_TEXT} characters: use dom with a 'root' selector for a smaller part"
    ));
    c
}

fn render(action: &str, r: &Value) -> String {
    match action {
        "list_browsers" => {
            let a = r.as_array().cloned().unwrap_or_default();
            let lines: Vec<String> = a
                .iter()
                .map(|b| {
                    format!(
                        "#{} {}\"{}\" {}",
                        b["browserId"],
                        if b["active"] == true { "(active) " } else { "" },
                        crate::perceive::clip(b["title"].as_str().unwrap_or(""), 60),
                        b["url"].as_str().unwrap_or("")
                    )
                })
                .collect();
            format!("{} browser(s)\n{}", a.len(), lines.join("\n"))
        }
        "list_tabs" => {
            let a = r.as_array().cloned().unwrap_or_default();
            let lines: Vec<String> = a
                .iter()
                .map(|t| {
                    format!(
                        "tab {}{} \"{}\" {}",
                        t["id"],
                        if t["active"] == true { " (active)" } else { "" },
                        crate::perceive::clip(t["title"].as_str().unwrap_or(""), 60),
                        t["url"].as_str().unwrap_or("")
                    )
                })
                .collect();
            format!("{} tab(s)\n{}", a.len(), lines.join("\n"))
        }
        "tab_info" => format!(
            "tab {} \"{}\" {} ({})",
            r["id"],
            crate::perceive::clip(r["title"].as_str().unwrap_or(""), 80),
            r["url"].as_str().unwrap_or(""),
            r["status"].as_str().unwrap_or("?")
        ),
        "dom" | "ax_tree" => cut(match r {
            Value::String(x) => x.clone(),
            Value::Object(o) => o
                .get("html")
                .or_else(|| o.get("tree"))
                .or_else(|| o.get("text"))
                .and_then(Value::as_str)
                .map(String::from)
                .unwrap_or_else(|| r.to_string()),
            _ => r.to_string(),
        }),
        _ => {
            let t = match r {
                Value::String(x) => x.clone(),
                Value::Null => String::new(),
                _ => r.to_string(),
            };
            cut(format!(
                "ok browser {action}{}",
                if t.is_empty() {
                    String::new()
                } else {
                    format!(": {}", crate::perceive::clip(&t, 300))
                }
            ))
        }
    }
}

/// The `browser` method.
pub fn run(p: &Value) -> Result<Value, ActError> {
    crate::safety::check()?;
    let (wire, payload, timeout) = plan(p)?;
    let action = s(p, "action").unwrap_or("");
    let r = request(wire, payload, timeout)?;
    Ok(json!({"text": render(action, &r)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handshake_vector_and_base64() {
        // RFC 6455 §1.3 example.
        assert_eq!(
            ws::accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
        assert_eq!(ws::base64(b"f"), "Zg==");
        assert_eq!(ws::base64(b"fo"), "Zm8=");
        assert_eq!(ws::base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(
            ws::sha1(b"abc")
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            "a9993e364706816aba3e25717850c26c9cd0d89d"
        );
    }

    #[test]
    fn frames_roundtrip_all_lengths_masked_and_not() {
        for n in [0usize, 5, 125, 126, 1000, 65535, 65536, 200_000] {
            let data: Vec<u8> = (0..n).map(|i| (i % 251) as u8).collect();
            for mask in [None, Some([1, 2, 3, 4])] {
                let f = ws::encode_frame(ws::OP_TEXT, &data, mask);
                let (fin, op, p) = ws::read_frame(&mut &f[..]).unwrap();
                assert!(
                    fin && op == ws::OP_TEXT && p == data,
                    "len {n} mask {mask:?}"
                );
            }
        }
    }

    #[test]
    fn fragmented_message_and_ping() {
        let mut buf = Vec::new();
        let mut a = ws::encode_frame(ws::OP_TEXT, b"hel", None);
        a[0] &= 0x7f; // not final
        buf.extend(a);
        buf.extend(ws::encode_frame(ws::OP_PING, b"x", None));
        buf.extend(ws::encode_frame(ws::OP_CONT, b"lo", None));
        let mut pings = 0;
        let mut on_ping = |_: &[u8]| -> io::Result<()> {
            pings += 1;
            Ok(())
        };
        let m = ws::read_message(&mut &buf[..], &mut on_ping)
            .unwrap()
            .unwrap();
        assert_eq!(m, (ws::OP_TEXT, b"hello".to_vec()));
        assert_eq!(pings, 1);
        let close = ws::encode_frame(ws::OP_CLOSE, b"", None);
        assert!(ws::read_message(&mut &close[..], &mut |_| Ok(()))
            .unwrap()
            .is_none());
    }

    #[test]
    fn plan_maps_and_refuses() {
        let (w, p, _) = plan(&json!({"action": "click", "text": "Send", "tab_id": 4})).unwrap();
        assert_eq!(w, "CLICK_ELEMENT");
        assert_eq!(p, json!({"selector": "text=Send", "tabId": 4}));
        let (w, p, _) = plan(&json!({"action": "type", "selector": "#q", "text": "hi"})).unwrap();
        assert_eq!((w, p["text"].as_str()), ("TYPE_TEXT", Some("hi")));
        assert_eq!(
            plan(&json!({"action": "execute_script", "script": "1"}))
                .unwrap_err()
                .code,
            "BLOCKED_BY_POLICY"
        );
        assert_eq!(
            plan(&json!({"action": "manage_cookies"})).unwrap_err().code,
            "BLOCKED_BY_POLICY"
        );
        assert_eq!(
            plan(&json!({"action": "navigate", "url": "javascript:alert(1)"}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
        assert_eq!(
            plan(&json!({"action": "navigate", "url": "file:///C:/x"}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
        assert!(plan(&json!({"action": "navigate", "url": "https://example.com"})).is_ok());
        assert_eq!(
            plan(&json!({"action": "click", "selector": "a", "text": "b"}))
                .unwrap_err()
                .code,
            "INVALID_ARGS"
        );
        assert_eq!(
            plan(&json!({"action": "nope"})).unwrap_err().code,
            "INVALID_ARGS"
        );
    }

    #[test]
    fn policy_helpers() {
        assert!(sensitive_selector("input[type=password]"));
        assert!(sensitive_selector("#otp-code"));
        assert!(!sensitive_selector("#search"));
        assert_eq!(
            element_name(&json!({"selector": "button[aria-label='Send']"})).as_deref(),
            Some("Send")
        );
        assert_eq!(
            element_name(&json!({"text": "Buy now"})).as_deref(),
            Some("Buy now")
        );
        assert_eq!(
            host_of("https://web.whatsapp.com/send?x=1"),
            "web.whatsapp.com"
        );
        assert!(cut("x".repeat(MAX_TEXT + 10)).contains("cut at"));
    }
}
