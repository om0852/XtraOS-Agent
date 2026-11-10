# ADR 0048 — P6-M5 Browser and Electron page content: settle the first read, reveal a minimized window

- Status: accepted (owner: "Yes, P6-M5", 2026-11-10).
- Date: 2026-11-10

## Context
docs/09-next-phases-plan.md §2.6: `observe` returned a 3-element "opaque" result where a full
list was needed, and Chrome page content was missing "until the window was in front".

Spike, 2026-11-10 (Windows 11 26200; Edge 156 InPrivate and Chrome incognito, both with
throw-away profiles and a local page of 60 links + 20 buttons; the user's running VS Code and
Chrome read-only, counts only):

| Case | First `observe` | Later |
|---|---|---|
| Edge / Chrome just opened, in front | 17 / 15 elements, no `Document` | 100 / 98 from +300 ms |
| Covered by another window before the first read | 15, no `Document` | 92 from +500 ms |
| **Minimized** before the first read | 15, no `Document` | **15 for as long as it stays minimized** |
| Minimized, then `ShowWindow(SW_SHOWNOACTIVATE)` | — | 98 within 150 ms; still 98 after `SW_SHOWMINNOACTIVE` |
| User's VS Code (Electron), first read in this core | 3, `opaque` | 257 at +400 ms |
| User's Chrome (normal state, fully loaded) | 32, no `Document` | 32 — not explained (see below) |

So Chromium (Chrome, Edge, Electron apps such as VS Code and Slack) builds its page tree only
after the first UI Automation request, and only while the window is not minimized. Covered
windows are fine. WM_GETOBJECT to the window and an MSAA client request did not change the
user's Chrome; current Chrome has no `Chrome_RenderWidgetHostHWND` child to target.

## Decision
1. **Settle the first read.** For a window whose class starts with `Chrome_WidgetWin_`, when a
   snapshot has no `Document` element, `observe`, `find` and `read_text` read again every
   150 ms, for at most 1.5 s, until one appears. A window still without a page after the full
   wait is remembered and not waited for again (a Chromium window without web content, e.g. a
   small popup, costs the wait once); the memory is dropped when a page appears. A minimized
   window is not waited for (its page never comes). `wait_for` is unchanged (it polls anyway).
   `observe` reports `waited_ms` when it waited.
2. **Say why the page is missing.** When a Chromium window still has no `Document`, `observe`
   returns `page: "minimized"` or `page: "not_exposed"` (else `null`) and the footer says what
   to do instead of only "opaque".
3. **`observe {reveal: true}`** (opt-in, contract change): for a minimized Chromium window with
   no page tree, XtraOS shows the window without activating it (`SW_SHOWNOACTIVATE`), waits up to
   1.5 s for the page, then minimizes it again without activating (`SW_SHOWMINNOACTIVE`) — also
   on errors. Focus and the user's input are never touched; the window is visible for a moment.
   It never applies to XtraOS's own or elevated windows (target checks run first) and is a no-op
   for other windows. Opt-in because the user can see it.

## Not decided / open
- The user's own long-running Chrome exposed no page while normal and loaded. Not reproduced with
  a fresh Chrome. Possible causes: Chrome's accessibility auto-disable, a sleeping tab, or the
  window's tab being a special page. Later the same day the same window gave 94 elements with its
  page on a first read by a fresh core, with nothing done to it (built code, `waited_ms` 0);
  VS Code gave 257. So it was a passing state of that window, not a permanent block. Until it is
  explained, `page: "not_exposed"` and the hint
  (focus the window once, or screenshot/ocr) are the answer. Recorded as open in P6-M5.
- Slack is not installed here; it is Electron like VS Code, so it is "planned" until benched.

## Tests
- Unit: the class rule (`Chrome_WidgetWin_1` yes, `Notepad` no) and the footer text.
- Interactive `p6m5_e2e` (XTRAOS_TEST_BROWSERS=1, throw-away profiles): a just-opened Edge and
  Chrome page is complete on the **first** `observe`; a minimized one reports `page: "minimized"`,
  then `reveal: true` returns the page, the window ends minimized and the foreground window is
  unchanged.
