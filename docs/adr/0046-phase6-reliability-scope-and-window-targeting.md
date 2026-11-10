# ADR 0046 — Phase 6 "Reliable daily driver": scope; P6-M1 window targeting, P6-M2 focus truth, P6-M3 clearer argument errors

- Status: accepted (owner: "move to the next phase / reliability", 2026-11-10). M1–M3 built.
- Date: 2026-11-10

## Context
Phase 5 (ADR 0044) is partly done: P5-M3 and P5-M7 are built; P5-M1/M2/M4/M8 wait for owner
decisions (publishing, signing, HTTP endpoint), P5-M5/M6/M9 are open. The owner asked to move
on to reliability. `docs/09-next-phases-plan.md` §2 lists failures seen in a long real session.
This ADR starts Phase 6 with the three cheapest, highest-impact ones:

1. `window` was documented as "title substring or ref" (contract, Conventions) but only refs
   worked: `window must be a ref like "w123"`.
2. A saved ref (`w2294892`) outlived its window. After Chrome restarted, Windows gave the same
   handle to a Firefox window, and `focus_window` focused Firefox with no warning.
3. `focus_window` said `focused: true` although another window took the foreground a moment
   later.
4. Argument mistakes (`keys: "ctrl+t"`, `target:` instead of `ref:`) got a hint that points at
   a doc file instead of showing a correct call.

## Decision
### P6-M1a: `window` accepts text
Everywhere a `window` argument is resolved (`winmgr::resolve_window`, so input, perception,
window tools, policy checks and the recorder all agree), a string that is not a ref, an integer
or `"desktop"` is matched against the windows `list_windows` shows (never XtraOS's own windows):

- case-insensitive; the title contains the text, or the process name equals it (with or
  without `.exe`);
- an exact title match wins over partial matches;
- exactly one match → that window; none → `WINDOW_NOT_FOUND`; several → `INVALID_ARGS` whose
  message lists the candidates (up to 5) as `ref "title"` so the agent can pick one.

Text never widens what an agent may touch: the policy engine and the elevation boundary act on
the resolved window, exactly as with a ref.

### P6-M1b: a reused handle is `STALE_REF`
The core remembers, for every window ref it hands out, the owning process id and that
process's start time (the pair identifies one process instance, as in ADR 0041). When a ref or
handle is used and the remembered pair differs from the window's current one, the call fails
with `STALE_REF` ("the window closed; its handle now belongs to firefox.exe …") and nothing is
touched. Listing windows again hands out the ref afresh. Limits, stated plainly:
- a handle the core never handed out (an integer from elsewhere) is accepted as before;
- the table lives in memory: after a core restart old refs are not detected;
- a new window of the *same* process reusing the handle is not detected.

### P6-M2: `focus_window` tells the truth
After the window comes to the front, the core waits `verify_ms` (default 300, 0 = off, max
2000) and looks again. The result has `focused` (still in front) and, when it lost the
foreground, `taken_by` with the ref, process and title of the window that has it. Bringing a
window forward stays best effort; the agent now knows when it failed.

### P6-M3: clearer argument errors
- `press_keys` accepts `keys` as one string (`"Ctrl+S"`) as well as an array.
- An `INVALID_ARGS` from a tool carries a hint with one correct example call for that tool
  instead of the pointer to the contract file.
- The MCP layer's "unexpected argument" error lists the arguments the tool does accept.

No other aliases are added: silently accepting wrong names hides mistakes.

### Phase 6, remaining milestones (unchanged from `docs/09`)
P6-M4 dialog awareness · P6-M5 browser/Electron content in the background · P6-M6 work
beside the user v2 (spike + ADR) · P6-M7 `wait`, `notify`, internal browser pages · P6-M8
large lists · P6-M9 close the open human-run gates.

## Consequences
- Contract: the `window` convention is now true; `focus_window` gains `verify_ms` and
  `taken_by`; `press_keys.keys` accepts a string; `STALE_REF` also covers a reused window handle.
- `focus_window` takes up to 300 ms longer by default.
- Tests: unit tests for matching and identity; interactive `win06` (title targeting on the
  testbed, ambiguous match, focus result); MCP contract test for the argument list.
