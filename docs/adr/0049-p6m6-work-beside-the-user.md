# ADR 0049 — P6-M6 Work beside the user: what runs in the background, and borrowing the foreground politely

- Status: accepted (owner: borrowing "On by default", 2026-11-10). Built; see §Build findings.
- Date: 2026-11-10

## Context
docs/09-next-phases-plan.md §2.8: web forms that needed real clicks failed whenever the user was
using another window — the largest time loss of the session. ADR 0014 already set the ladder:
UI Automation patterns first (no mouse, no keys), polite real input only when the user is idle.
The question here: on a real web form in a browser that sits behind the user's window, which
steps work in the background, which silently fail, and what to do with the rest.

## Spike (2026-11-10)
Local test page (`<input>`, a framework-style input whose state changes only on `input` events,
checkbox, radio, `<select>`, custom ARIA dropdown, `contenteditable`, `role=button` div, plain
`<div onclick>`, link, Submit). Its scripts write their state into the title, so we see what the
**page** noticed, not only what UIA reports. Edge InPrivate and Chrome incognito with throw-away
profiles; the TopMost testbed in front as the user's window. The user was using the PC during
parts of the runs; foreground/cursor changes in those parts were the user's (idle 0 ms), and
only hands-off readings are used below.

| Step (browser behind the user's window) | XtraOS today | Page noticed |
|---|---|---|
| `set_value` text input | background (Value) | yes, `input` event |
| `set_value` framework-style input | background | yes |
| `click` checkbox / radio / link / `role=button` div | background (Toggle / Invoke) | yes |
| `click` custom dropdown, then its option | background (Expand, Select) | yes |
| `click` `<select>` (expand), then option | background (Expand, Select) | yes |
| `set_value` `<select>` | **"ok"** | **no — silent no-op** |
| `set_value` `contenteditable` | **"ok"**, reads back the text | **no `input` event** — frameworks keep the old state |
| `click` plain `<div onclick>` (no pattern; UIA shows only its Text) | real input → `TARGET_OBSCURED` (covered) | — |
| `type` | `NEEDS_HUMAN` (not foreground) | — |
| `click` "Submit" | policy confirm dialog (by design, ADR 0017) | — |

Ways around the real-input steps, tested with a managed UIA client:
- UIA `SetFocus` on a page element **activates the browser window** (Edge and Chrome) — it is a
  foreground change, not a background action.
- `WM_CHAR` posted to the browser window types into the focused field (`input` events fire)
  **only while the browser is the active window**. With the browser behind the user's window
  (hands-off, idle ≈ 1 s), neither characters nor Backspace (`WM_KEYDOWN`) arrived (Edge).
  So there is no background typing into Chromium.
- MSAA default action (LegacyIAccessible `DoDefaultAction`) for the plain div: not testable with
  the managed client (it has no LegacyIAccessible); to test with the native client in the build.

Classic desktop apps: ADR 0014 spike S10 and P1-BG tests already show Invoke/Toggle/Value work in
the background (Calculator, WinForms); text that needs keystrokes has the same limit as browsers.

## Decision (proposed)
1. **No silent failures.**
   - `set_value` on a ComboBox whose value is not writable but which can expand (HTML `<select>`,
     many desktop combos): expand, select the option whose name equals the value, collapse —
     all background (proven above). No such option → `ELEMENT_NOT_FOUND` listing the options.
     Then read the value back; a mismatch is an error, never "ok".
   - `set_value` on a browser rich-text field (Chromium Edit that is not an `<input>`/`<textarea>`;
     detection to be confirmed in the build): result says `mode=background, verify: true` with
     "rich-text editors may ignore this; check the page or use type".
2. **One more background rung for clicks:** before real input, try the native MSAA default action
   (`IUIAutomationLegacyIAccessiblePattern::DoDefaultAction`) on the element and, for a Text
   element, on its parent. Kept only if a test shows the page's handler runs.
3. **Borrow the foreground politely** for what must be real input (typing, plain divs, canvas):
   - wait until the user has been idle ≥ 1.5 s (up to `timeout_ms`, default 30 s — today 5 s);
   - bring the target window to the front, do one short burst, then put the user's window back in
     front and the cursor back (focus and cursor restore already exist for clicks);
   - any user input during the burst aborts it (existing guards), and the user's window gets the
     foreground back at once;
   - a burst longer than 3 s (long text) first asks via `ask_human` ("XtraOS needs the keyboard
     for about N s"), unless the user said yes for this window in the last 10 minutes;
   - results report `borrowed_ms`. **Owner decision needed:** on by default for `type`,
     `press_keys` and `click`, or only with `borrow: true`.
4. **Not doing:** posting keystrokes to background windows (does not work for Chromium and could
   land in the wrong element); hidden desktops or second sessions (ADR 0014 §3).

## Build findings (2026-11-10)
- **Owner decision:** borrowing is on by default for `type`, `press_keys` and real `click`;
  `borrow: false` keeps the old refusal (`NEEDS_HUMAN`); `focus: true` keeps its old meaning
  (bring the window up and leave it there).
- **`type {ref, text}`** was added: the field is focused through UIA after the window is up,
  password fields are refused (by the ref and by the focused element).
- **Plain clickable `<div>`:** UIA shows it as a `Text` whose parent `Group` has Invoke. `click`
  on a Text/Image now tries the direct parent (Group/Custom/Pane/ListItem/DataItem) before real
  input: `via: parent_invoke`. The MSAA default action was already a rung of `click`.
- **`<select>`:** its options exist only while it is open (combo > List > ListItem; neither
  `FindAll` nor SelectionPattern reached them), and opening it **activates the browser**. Giving
  the foreground back right after opening closed the popup, so open → select → close → read back
  now runs inside one foreground hold, after waiting until the user is idle (1.5 s) like a borrow.
  The window is in front for about half a second.
- **Rich-text vs `<input>`:** identical in every UIA property read (role, class, framework,
  patterns). `set_value` on any browser text field adds a `note` that rich-text editors may not
  notice and `type {ref, text}` does.
- The user-interruption abort (`Borrow::check`) relies on the safety hooks' input time; it is
  not covered by an automated test (it needs real user input). The "ask first" prompt for bursts
  over 3 s is unit-tested for when it triggers, not clicked in a test.

## Tests (built)
- Core unit: `long_bursts_ask_first_short_ones_do_not`, `options_match_exactly_then_loosely`,
  argument checks for `type` (`ref` / `window`, `borrow`).
- Interactive `p6m6_e2e` (XTRAOS_TEST_BROWSERS=1), Edge and Chrome behind the TopMost testbed:
  the page saw `<select>` = Japan with a change event, the name input, the plain div click
  (`parent_invoke`) and "hello" typed into the contenteditable with input events; borrowing took
  ≈ 340–350 ms and the testbed was in front again afterwards; `borrow:false` → `NEEDS_HUMAN`;
  an unknown option → `ELEMENT_NOT_FOUND` listing the options.

## Tests (planned in the proposal)
- `p6m6_e2e` (browsers opt-in): the form page behind the testbed; `<select>` via `set_value`
  changes the page; contenteditable returns `verify`; plain div via the MSAA rung (if kept);
  `type` with borrow types into the page and the testbed is in front again afterwards; user input
  during a borrow aborts it.
- Unit: option matching, borrow timing rules.
