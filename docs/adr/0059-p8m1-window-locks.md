# ADR 0059 — P8-M1 Several clients at once: per-window locks with a fair queue

- Status: accepted (owner: "Keep building", 2026-11-11; plan row P8-M1).
- Date: 2026-11-11

## Context
XtraOS is set up for several agent clients at once (Claude Code and Codex are both connected on
the owner's PC). Each connection is its own session (`s1`, `s2`, …) and real input is already
serialised process-wide (`input::action_lock`), but two agents can still interleave on the **same
window**: one types into a form while the other clicks in it. Nothing tells an agent that
another one is working there.

## Decision
1. **Window leases.** An *acting* call (click, type, press_keys, scroll, drag, mouse, set_value,
   touch, pen, focus_window, close_window, window_bounds) from a session takes a lease on every
   window it targets (`window`/`to_window`, the window of `ref`/`from_ref`/`to_ref`, the window
   under `x,y`). The lease belongs to that session while it keeps acting; it lapses 20 s after
   the session's last action there, and at once when the session disconnects.
2. **Fair queue.** Another session acting on a leased window waits in a first-come queue for up
   to 10 s; when the window frees, the head of the queue gets it. Still busy after 10 s →
   `WINDOW_BUSY` "in use by <client> (session sN), last action K s ago" (new error code).
3. **Reading is never blocked**: observe, find, read_text, screenshot, ocr, wait_for, … run
   alongside. The **user** is never blocked by leases (their input is not an agent session).
4. **`status`** gets `locks`: `[{window, title, client, session, idle_s}]` and each client sees
   its own session id, so agents can tell who holds what.
5. Leases live in memory only; a core restart clears them.

## Tests
- Unit: the lease/queue state machine with a fake clock (take, renew, lapse, FIFO hand-over,
  timeout, release on disconnect).
- Interactive: two paired clients on one core and one testbed — client A clicks (holds the lease),
  client B's click waits and then gets `WINDOW_BUSY` (short test wait), B's `observe` works
  meanwhile; A disconnects → B's click goes through; `status.locks` shows A's lease.
