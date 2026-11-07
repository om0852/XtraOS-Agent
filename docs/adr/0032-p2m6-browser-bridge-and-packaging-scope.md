# ADR 0032 — P2-M6 scope: browser bridge to XtraDevPilot + packaging decision

- Status: accepted (scope). Findings and gate → ADR 0033.
- Date: 2026-11-07

## Context
UIA gives only coarse access to web pages; many controls lack names or roles. XtraDevPilot
(the owner's Chrome extension + MCP bridge) already drives pages by CSS selector or visible
text, with trusted input. Its bridge (commit 2a4e51b):
- listens on `ws://127.0.0.1:42819`;
- refuses web origins;
- requires the pairing token from `%USERPROFILE%\.xtradevpilot\bridge-token` for
  non-extension clients;
- relays `{id, action, …}` to the extension and returns `{id, result | error}`.

ADR 0021 also leaves packaging (MSI) and signing to this milestone.

## Decision
### A. The core is the bridge client (not the MCP server)
Browser actions then pass the same safety path as everything else: policy, approval
dialogs, kill switch, overlay and the action log.
- `browser.rs`:
  - a minimal RFC 6455 WebSocket client (std TCP, no new dependency);
  - connects only to the loopback address (127.0.0.1), with no Origin header;
  - checks the handshake `Sec-WebSocket-Accept`;
  - authenticates with an `AUTH` message carrying the token (never in the URL) and
    requires `AUTH_RESULT ok`;
  - masks every client frame; messages up to 64 MB; replies to ping;
  - keeps one connection and reconnects once on error.
- Port and token: `XTRAOS_BROWSER_PORT` (default 42819) and the XtraDevPilot token file,
  or the `XTRADEVPILOT_TOKEN` environment variable.
- The bridge must be running (the XtraDevPilot MCP server starts it), with the extension
  loaded in Chrome. Otherwise: `NEEDS_HUMAN` with a hint.

### B. One tool, `browser {action, …}`, with an allowlist
| action | wire | class |
|---|---|---|
| `list_browsers`, `list_tabs`, `tab_info` | LIST_BROWSERS, LIST_TABS, GET_TAB_INFO | observe |
| `dom` (clean DOM, optional `root`), `ax_tree` | GET_CLEAN_DOM, GET_AX_TREE | observe |
| `wait_for` (selector, state) | WAIT_FOR_ELEMENT | observe |
| `click` (selector or text) | CLICK_ELEMENT | input (+ send/buy/delete words) |
| `type` (selector, text) | TYPE_TEXT | input (+ password/OTP/card fields → denied) |
| `press_key` (key, selector?) | PRESS_KEY | input (+ Enter on a messaging site → confirm) |
| `scroll`, `navigate`, `open_tab`, `switch_browser` | SCROLL_PAGE, NAVIGATE, OPEN_TAB, SWITCH_BROWSER | input |

- **Not exposed** (confidentiality / integrity): `EXECUTE_SCRIPT` (arbitrary JavaScript in
  signed-in tabs), cookies and storage (session tokens), network mocks, HAR, source
  download, file upload/drop, extension reload. They remain available in XtraDevPilot's
  own MCP for development work, outside XtraOS.
- **Policy facts** come from the target tab (GET_TAB_INFO, 3 s) for input actions: the host
  and title feed the existing rules.
  - Enter or a `send` click on WhatsApp, Telegram, Slack, Teams, Discord, Messenger,
    Gmail or Outlook web → `message_send`, which confirms.
  - Buy/pay words → `purchase`, which is denied.
  - Delete words → `delete`, which confirms.
  - A selector naming a password, OTP, CVV or card field → `credential_entry`, which is
    denied.
- Results are compact text: DOM/AX output is passed through, limited to 60,000 characters.

### C. Packaging and signing
- **No MSI for now.** WiX v4+ asks organisations to pay its Open Source Maintenance Fee;
  older free toolchains are unmaintained. The owner's rule is no paid services. The zip,
  `install.ps1` and `uninstall.ps1` (ADR 0019) remain the package.
- **Signing stays an owner decision** (ADR 0019: unsigned developer preview). The installer
  documents it.
- `INSTALL.md` gets the browser-bridge prerequisites.

## Tests (docs/08 P2-BR-*)
- **Automated, no human:**
  - WebSocket codec unit tests: RFC 6455 handshake vector, masking, lengths 125/126/127+,
    fragmentation;
  - a fake bridge in the test (raw TCP): wrong token → `NEEDS_HUMAN`; good token →
    actions mapped to the wire names; refused actions; Enter on `web.whatsapp.com` asks
    (the test answers Deny); a password selector is denied; the kill switch stops it; a
    reply over 60,000 characters is cut.
- **Live, opt-in (`XTRAOS_TEST_BRIDGE=1`):** `list_browsers` against the running XtraDevPilot
  bridge (read only; prints counts, not URLs).

## Consequences
- docs/04: `browser` tool, 29 tools.
- docs/05: browser rules.
- docs/08: P2-BR cases.
- INSTALL.md: prerequisites.
