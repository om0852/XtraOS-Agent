# ADR 0004 — Screen capture mapping (S3) and pipe pairing (S6) findings

- Status: accepted
- Date: 2026-11-07

## S3 — capture → screen mapping

Setup: per-monitor DPI aware v2 thread, our own non-activating test window with a magenta
marker; marker found in the capture and compared with `ClientToScreen`.

| Monitor | Bounds | DPI | PrintWindow(PW_RENDERFULLCONTENT) | Screen copy (BitBlt) |
|---|---|---|---|---|
| 1 | 0,0 1920×1080 | 144 (150%) | delta (0,0) | delta (0,0) |

Decision:
- With per-monitor-v2 awareness, window rect, `ClientToScreen`, captures and cursor
  coordinates are all **physical pixels** and agree exactly → the core owns all mapping;
  the agent never does DPI maths (the mis-clicks seen with 1.35×/1.5× browser scaling are
  avoided by design).
- `GetWindowRect` includes the invisible resize border (seen as black edges in PrintWindow
  output) → crop with `DwmGetWindowAttribute(DWMWA_EXTENDED_FRAME_BOUNDS)`.
- Only one monitor was available; multi-monitor / mixed-DPI remains to be tested.
- Windows.Graphics.Capture is re-validated in Rust (ADR 0002).

## S6 — named pipe + token pairing

DACL produced: `D:(D;;0x1f019f;;;NU)(A;;0x12019f;;;<current-user-SID>)` → only the current user
may connect; network logons explicitly denied.

| Test | Result |
|---|---|
| Correct token, 500 pings (Node client) | paired; **p50 0.140 ms, p95 0.221 ms, max 0.815 ms** |
| Wrong token | `NOT_PAIRED`, connection closed |
| No token | `NOT_PAIRED`, connection closed |

Decision: named pipe + user-only DACL + NETWORK deny + 256-bit token (constant-time compare)
is the core↔MCP transport. Pipe latency is ~14× under the 2 ms budget. In Rust also set
`PIPE_REJECT_REMOTE_CLIENTS`. Cross-user denial was not testable on a single-user machine
(the DACL is the evidence).
