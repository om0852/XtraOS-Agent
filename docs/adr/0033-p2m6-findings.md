# ADR 0033 — P2-M6 Browser bridge + packaging: findings and gate result

- Status: accepted (built, automated tests pass). The live check needs an XtraDevPilot bridge
  with the pairing token (see finding 1).
- Date: 2026-11-07

## What was built (ADR 0032)
- `browser.rs`:
  - a minimal RFC 6455 client with no new dependency (SHA-1, base64, framing, ping/pong,
    fragments, 64 MB limit);
  - loopback-only connect, handshake check, AUTH-message token, one reused connection;
  - the `browser` action allowlist with wire mapping;
  - http/https-only navigation;
  - refused actions (scripts, cookies, storage, uploads, HAR, mocks, source download);
  - compact rendering, cut at 60,000 characters.
- Policy:
  - for input actions, the target tab's host and title (GET_TAB_INFO) feed the existing
    rules as a `chrome.exe` window;
  - messaging Enter or Send → `message_send` (confirm);
  - buy/pay → `purchase` (deny);
  - password/OTP/card selectors → `credential_entry` (deny);
  - `browser:*` read actions are class observe, the rest input.
- `browser` is allowed inside `batch`. MCP tool `browser` (29 tools). INSTALL.md lists the
  prerequisites.
- Packaging: zip + `install.ps1` stays; no MSI (WiX v4+ maintenance fee conflicts with the
  no-paid rule); signing remains the owner's decision (ADR 0019).
- Gate: `tests/phase2/p2m6-gate.ps1` (`-Automated`: P2-M5 helper + P2-M6 bridge, no human).

## Results
- Unit tests: 66, including the RFC 6455 handshake vector, SHA-1 "abc", every length class
  masked and unmasked, fragments with a ping in between, and close.
- P2-BR-01..05 against a fake bridge: 5/5.
  - A bad token → `NEEDS_HUMAN`, and nothing was relayed.
  - The refused actions never reached the bridge; it saw only the tab lookups.
  - Enter on web.whatsapp.com asked; Deny sent nothing, Allow sent.
  - After the kill switch: `STOPPED`, nothing sent.
- Gate (`-Automated`): **12/12 PASS**.

## Findings
1. **The XtraDevPilot bridge running on this PC is the older published build.**
   - It listens on `0.0.0.0:42819`, so the LAN can reach it, and it has no pairing token.
   - The security fix (commit 2a4e51b, loopback + token) is not in the running copy yet.
   - XtraOS correctly refuses to use it (`NEEDS_HUMAN`: no pairing token), so the live
     check fails as designed.
   - **Action for the owner:** publish/run an XtraDevPilot build that includes 2a4e51b,
     then run P2-BR-LIVE (`XTRAOS_TEST_BRIDGE=1`).
2. **Every input action costs one extra tab lookup.** The lookup gives the policy its facts
   and is cheap on loopback. If the tab cannot be read, the action still runs with the
   desktop-level rules only; the dialog then names no site.

## Open
- P2-BR-LIVE with the patched bridge.
- Clicks on cross-origin iframes and shadow DOM depend on XtraDevPilot's selector support
  (`>>>`). They are not tested here.
