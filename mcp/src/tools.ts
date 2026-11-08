// The MCP tool surface. MUST match docs/04-mcp-tool-contract.md (a contract test enforces it).
// M1: status. M2 (ADR 0010): click, type, press_keys, scroll, drag, mouse, clipboard.
// M3 (ADR 0012): list_windows, focus_window, launch_app, close_window, window_bounds.
// M4 (ADRs 0014, 0015): observe, find, read_text, screenshot, set_value; `ref` targets act in the
// background first (no cursor, no keys); real input waits for the user to be idle.
// M5 (ADR 0017): ask_human, wait_for_human; every action passes the core's policy engine and
// kill switch (BLOCKED_BY_POLICY, STOPPED — only the user resumes, from the tray).
// P2-M1 (ADR 0022): wait_for, batch, observe(scope_ref) — fewer agent round trips.
// P2-M2 (ADR 0024): ocr — text with screen coordinates for opaque windows.
// P2-M3 (ADR 0026): app_map, list_skills, run_skill.
// Coordinates are physical screen pixels; `window` is a ref like "w460650" or the integer handle.
import type { CoreClient } from './core-client.ts';

type Schema = { type: 'object'; properties: Record<string, unknown>; required?: string[]; additionalProperties: false };

export type Content =
  | { type: 'text'; text: string }
  | { type: 'image'; data: string; mimeType: string };

export type ToolDef = {
  name: string;
  description: string;
  inputSchema: Schema;
  run: (core: CoreClient, args: Record<string, unknown>) => Promise<string | Content[]>;
};

const int = (description: string) => ({ type: 'integer', description });
const WINDOW = {
  type: ['string', 'integer'],
  description: 'Window ref from list_windows (e.g. "w460650") or its integer handle, or "desktop" for the desktop icon view. Mouse: optional guard (point must be inside it). Keyboard: required and must be the foreground window.',
};
const WINDOW_REQ = { type: ['string', 'integer'], description: 'Window ref from list_windows (e.g. "w460650") or its integer handle, or "desktop" for the desktop icon view' };

type Win = { ref: string; process: string; title: string; x: number; y: number; w: number; h: number; state: string; focused: boolean; elevated: boolean; agent?: boolean; reused?: boolean };
/** One compact line per window (docs/06 token budget). */
const winLine = (w: Win) =>
  `${w.ref} ${w.process} "${w.title.length > 80 ? w.title.slice(0, 79) + '…' : w.title}" ${w.x},${w.y} ${w.w}x${w.h}` +
  `${w.state !== 'normal' ? ' ' + w.state : ''}${w.focused ? ' focused' : ''}${w.elevated ? ' elevated' : ''}${w.agent ? ' agent' : ''}`;
const X = int('Screen X in physical pixels');
const Y = int('Screen Y in physical pixels');
const BUTTON = { type: 'string', enum: ['left', 'right', 'middle'], description: 'Mouse button (default left)' };
const REF = { type: 'string', pattern: '^e[0-9]+$', description: 'Element ref from observe/find, e.g. "e12"' };
/** Polite real input (ADR 0014): wait for the user, put the cursor back. */
const IDLE = { type: 'integer', minimum: 0, maximum: 60000, description: 'Real input waits until the user has been idle this long (default 800 ms, 0 = off)' };
const TIMEOUT = { type: 'integer', minimum: 0, maximum: 120000, description: 'Max wait for the user to be idle (default 5000 ms), then USER_ACTIVE' };
const RESTORE = { type: 'boolean', description: 'Put the cursor back afterwards (default true)' };
/** "ok click e12 · background (invoke)" / "ok click ×1 at (x,y) · input" */
const how = (r: any) =>
  r.mode === 'background' ? ` · background (${r.via}${r.foreground_restored ? ', focus kept' : ''})` : ' · input';

type Status = { version: string; session: string; client: string; state: string; uptime_ms: number; kill_hotkey: string | null; admin_helper?: { running: boolean; pid?: number } };

/** Forwards to the core and renders a compact one-line result (docs/06: <= 30 tokens). */
const forward = (method: string, render: (r: any, a: Record<string, unknown>) => string) =>
  async (core: CoreClient, args: Record<string, unknown>) => render(await core.call(method, args), args);

export const TOOLS: ToolDef[] = [
  {
    name: 'status',
    description:
      'Shows whether the XtraOS core is running and paired: version, session, state (running / stopped by the ' +
      'user) and the kill-switch hotkey. Call this first; if it fails, ask the user to start the XtraOS core. ' +
      'If stopped: stop entirely — only the user can resume, from the XtraOS tray icon.',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
    async run(core) {
      const s = (await core.call('status')) as Status;
      return `XtraOS core ${s.version} · ${s.state} · session ${s.session} (${s.client}) · up ${Math.round(s.uptime_ms / 1000)}s` +
        (s.kill_hotkey ? ` · stop: ${s.kill_hotkey}` : ' · stop: tray icon') +
        (s.admin_helper?.running ? ` · admin helper on (pid ${s.admin_helper.pid})` : '');
    },
  },
  {
    name: 'click',
    description:
      'Clicks an element or a point. PREFER `ref` (from observe/find): a plain left click on a ref runs in the ' +
      'background through the element itself — no cursor move, no keystrokes, the user keeps working. ' +
      'Otherwise real mouse input at (x, y): it waits until the user is idle, refuses (TARGET_OBSCURED) if the point ' +
      'is not inside `window`, aborts (USER_ACTIVE) if the user moves the mouse, and puts the cursor back.',
    inputSchema: {
      type: 'object',
      properties: {
        ref: REF, x: X, y: Y, window: WINDOW, button: BUTTON,
        clicks: { type: 'integer', minimum: 1, maximum: 3, description: '1 = click, 2 = double, 3 = triple' },
        modifiers: { type: 'array', items: { type: 'string', enum: ['ctrl', 'shift', 'alt', 'win'] }, description: 'Keys held during the click' },
        idle_ms: IDLE, timeout_ms: TIMEOUT, restore_cursor: RESTORE,
      },
      additionalProperties: false,
    },
    run: forward('click', (r, a) =>
      r.mode === 'background' ? `ok click ${a.ref}${how(r)}` : `ok click ×${r.clicks} at (${r.at[0]},${r.at[1]})${how(r)}`),
  },
  {
    name: 'type',
    description:
      'Types unicode text (any language, emoji) with real keyboard input into `window`, which must be the ' +
      'foreground window (else NEEDS_HUMAN, nothing typed). Waits until the user is idle. Newline = Enter, tab = Tab. ' +
      'The text is never logged. To fill a field without the keyboard, prefer set_value(ref).',
    inputSchema: {
      type: 'object',
      properties: {
        window: WINDOW,
        text: { type: 'string', maxLength: 10000, description: 'Text to type' },
        submit: { type: 'boolean', description: 'Press Enter afterwards' },
        idle_ms: IDLE, timeout_ms: TIMEOUT,
      },
      required: ['window', 'text'],
      additionalProperties: false,
    },
    run: forward('type', (r) => `ok typed ${r.typed_chars} chars${r.submitted ? ' + Enter' : ''}`),
  },
  {
    name: 'press_keys',
    description:
      'Presses key chords in order with real keyboard input, e.g. ["Ctrl+S"], ["Ctrl+Shift+P"], ["Alt+F4"], ["Enter"]. ' +
      '`window` must be the foreground window (else NEEDS_HUMAN). Modifiers are always released.',
    inputSchema: {
      type: 'object',
      properties: {
        window: WINDOW,
        keys: { type: 'array', items: { type: 'string' }, minItems: 1, maxItems: 100, description: 'Chords, pressed in order' },
        idle_ms: IDLE, timeout_ms: TIMEOUT,
      },
      required: ['window', 'keys'],
      additionalProperties: false,
    },
    run: forward('press_keys', (r) => `ok ${r.chords} chord(s)`),
  },
  {
    name: 'scroll',
    description:
      'Scrolls. With `ref`: in the background — a scrollable list/pane scrolls by dy/dx lines, an item without ' +
      'dy/dx is scrolled into view. Otherwise the real mouse wheel at (x, y) (waits for the user to be idle). ' +
      'dy > 0 scrolls down, dx > 0 scrolls right (notches).',
    inputSchema: {
      type: 'object',
      properties: {
        ref: REF, x: X, y: Y, window: WINDOW,
        dy: { type: 'integer', minimum: -50, maximum: 50 },
        dx: { type: 'integer', minimum: -50, maximum: 50 },
        idle_ms: IDLE, timeout_ms: TIMEOUT, restore_cursor: RESTORE,
      },
      additionalProperties: false,
    },
    run: forward('scroll', (r, a) =>
      r.mode === 'background' ? `ok scroll ${a.ref}${how(r)}` : `ok scroll dy=${r.dy} dx=${r.dx}${how(r)}`),
  },
  {
    name: 'drag',
    description:
      'Real drag-and-drop: press at the start, move in steps, release at the end. Ends are element refs ' +
      '(from_ref / to_ref, e.g. a file in Explorer) or points (from_x/from_y, to_x/to_y). Works with OS ' +
      'drag-and-drop. `window` guards the start, `to_window` the drop point. Waits for the user to be idle; ' +
      'the button is released if anything fails.',
    inputSchema: {
      type: 'object',
      properties: {
        from_ref: REF, to_ref: REF,
        from_x: X, from_y: Y, to_x: X, to_y: Y, window: WINDOW, to_window: WINDOW, button: BUTTON,
        idle_ms: IDLE, timeout_ms: TIMEOUT, restore_cursor: RESTORE,
        steps: { type: 'integer', minimum: 1, maximum: 500, description: 'Intermediate moves (default 20)' },
        step_ms: { type: 'integer', minimum: 0, maximum: 200, description: 'Delay per step (default 12)' },
        hold_ms: { type: 'integer', minimum: 0, maximum: 2000, description: 'Pause after press and before release (default 60)' },
      },
      additionalProperties: false,
    },
    run: forward('drag', (r) => `ok drag (${r.from[0]},${r.from[1]}) → (${r.to[0]},${r.to[1]})`),
  },
  {
    name: 'mouse',
    description:
      'Low-level mouse: move, down, up at (x, y), or path through points [[x,y],...] (button held unless hold=false) ' +
      'for drawing, sliders and canvas apps.',
    inputSchema: {
      type: 'object',
      properties: {
        action: { type: 'string', enum: ['move', 'down', 'up', 'path'] },
        x: X, y: Y, window: WINDOW, button: BUTTON,
        points: { type: 'array', items: { type: 'array', items: { type: 'integer' }, minItems: 2, maxItems: 2 }, minItems: 2, maxItems: 2000 },
        hold: { type: 'boolean', description: 'path: hold the button while moving (default true)' },
        step_ms: { type: 'integer', minimum: 0, maximum: 200 },
        idle_ms: IDLE, timeout_ms: TIMEOUT,
        restore_cursor: { type: 'boolean', description: 'Put the cursor back after move/up/path (default false: gestures span calls)' },
      },
      required: ['action'],
      additionalProperties: false,
    },
    run: forward('mouse', (r, a) => (a.action === 'path' ? `ok path ${r.points} points` : `ok ${a.action} at (${r.at[0]},${r.at[1]})`)),
  },
  {
    name: 'observe',
    description:
      'Reads a window as compact lines with element refs: `e5 Button "Save"`, `e7 Edit "Search" = "text"`, ' +
      '`[x]` checked, `(off)` disabled, `***` password (never revealed). Default: the foreground window. A repeat ' +
      'call on the same window returns only what changed (+ added, - removed, ~ changed) unless full=true. ' +
      '`opaque` = few accessible elements (canvas/game): use screenshot. Refs work in click, scroll, drag, set_value.',
    inputSchema: {
      type: 'object',
      properties: {
        window: WINDOW_REQ,
        scope_ref: { ...REF, description: 'Only this element\'s subtree (a list, pane or dialog): fewer tokens on big windows' },
        full: { type: 'boolean', description: 'Full list instead of a diff' },
        max_elements: { type: 'integer', minimum: 1, maximum: 500, description: 'Default 120' },
      },
      additionalProperties: false,
    },
    run: forward('observe', (r) => r.text),
  },
  {
    name: 'find',
    description:
      'Finds elements in a window (default foreground) whose name, role or automation id contains `query`, ' +
      'including list rows scrolled out of view (marked offscreen); returns up to 20 refs. deep=true also searches ' +
      'virtualised lists (e.g. Explorer folders with thousands of files) through the list itself: an exact name is fast, ' +
      'a partial name scans rows for up to timeout_ms (default 3000). Such rows are marked virtualized and are brought ' +
      'into view only when you act on the ref. click(ref) on a list row selects it (it never opens the item).',
    inputSchema: {
      type: 'object',
      properties: {
        query: { type: 'string', minLength: 1 },
        window: WINDOW_REQ,
        deep: { type: 'boolean', description: 'Also search virtualised list rows (slower)' },
        timeout_ms: { type: 'integer', minimum: 0, maximum: 15000, description: 'deep: scan budget (default 3000)' },
      },
      required: ['query'],
      additionalProperties: false,
    },
    run: forward('find', (r) => r.text),
  },
  {
    name: 'read_text',
    description:
      'Reads text: of an element (`ref`: document, field, label) or of a window (its main document/editor, else ' +
      'its visible labels). Password fields return ***. Up to 20,000 characters; last_lines keeps the end.',
    inputSchema: {
      type: 'object',
      properties: {
        ref: REF, window: WINDOW_REQ,
        last_lines: { type: 'integer', minimum: 1, maximum: 10000 },
      },
      additionalProperties: false,
    },
    run: forward('read_text', (r) => (r.text === '' ? '(no text)' : r.text) + (r.truncated ? '\n-- truncated' : '')),
  },
  {
    name: 'screenshot',
    description:
      'Captures a window (default foreground; also when covered) or a screen region as PNG, downscaled ' +
      '(scale 0.5 default). Use it when observe says opaque. Maps back with screen = origin + pixel / scale.',
    inputSchema: {
      type: 'object',
      properties: {
        window: WINDOW_REQ,
        region: {
          type: 'object',
          properties: { x: X, y: Y, w: { type: 'integer', minimum: 1 }, h: { type: 'integer', minimum: 1 } },
          required: ['x', 'y', 'w', 'h'],
          additionalProperties: false,
          description: 'Screen region in physical pixels (instead of a window)',
        },
        scale: { type: 'number', minimum: 0.1, maximum: 1, description: 'Default 0.5' },
      },
      additionalProperties: false,
    },
    async run(core, args) {
      const r: any = await core.call('screenshot', args);
      return [
        { type: 'image', data: r.png_base64, mimeType: 'image/png' },
        {
          type: 'text',
          text: `${r.window ?? 'region'} ${r.width}x${r.height}px · screen = (${r.origin[0]},${r.origin[1]}) + px / ${r.scale}`,
        },
      ];
    },
  },
  {
    name: 'ocr',
    description:
      'Reads text from pixels with Windows\' built-in OCR (offline) — for opaque windows where observe ' +
      'finds few elements (games, canvas, custom-drawn apps). Returns lines `"text" @cx,cy (x,y wxh)` in ' +
      'screen pixels: click(x: cx, y: cy) hits the text. query keeps only matching lines (box of the ' +
      'matching words). Far fewer tokens than a screenshot.',
    inputSchema: {
      type: 'object',
      properties: {
        window: WINDOW_REQ,
        region: {
          type: 'object',
          properties: { x: X, y: Y, w: { type: 'integer', minimum: 1 }, h: { type: 'integer', minimum: 1 } },
          required: ['x', 'y', 'w', 'h'],
          additionalProperties: false,
          description: 'Screen region in physical pixels (instead of a window)',
        },
        query: { type: 'string', minLength: 1 },
        max_lines: { type: 'integer', minimum: 1, maximum: 500 },
        language: { type: 'string', description: 'Installed OCR language tag, e.g. en-US (default: user languages)' },
      },
      additionalProperties: false,
    },
    run: forward('ocr', (r) => r.text),
  },
  {
    name: 'set_value',
    description:
      'Sets the text of a field in the background (no keystrokes, no focus change) via UI Automation. ' +
      'Refuses password fields (NEEDS_HUMAN: the user types passwords). If the element has no settable value, ' +
      'use click(ref) + type.',
    inputSchema: {
      type: 'object',
      properties: { ref: REF, value: { type: 'string', maxLength: 10000 } },
      required: ['ref', 'value'],
      additionalProperties: false,
    },
    run: forward('set_value', (r, a) => `ok set ${a.ref} (${r.chars} chars)${how(r)}`),
  },
  {
    name: 'wait_for',
    description:
      'Waits in the core (no polling from you) until ALL given conditions hold: window_title contains text ' +
      '(without window: any window → its ref), text_contains (visible text or terminal/editor text of the ' +
      'window), ref_gone, window_gone, idle_ms (window unchanged that long). timeout_ms default 10 s → TIMEOUT.',
    inputSchema: {
      type: 'object',
      properties: {
        window: WINDOW_REQ,
        window_title: { type: 'string', minLength: 1 },
        text_contains: { type: 'string', minLength: 1 },
        ref_gone: REF,
        window_gone: { type: 'boolean' },
        idle_ms: { type: 'integer', minimum: 50, maximum: 60000 },
        timeout_ms: { type: 'integer', minimum: 100, maximum: 120000 },
      },
      additionalProperties: false,
    },
    run: forward('wait_for', (r) => `ok ${r.met.join(' + ')} after ${r.ms} ms${r.window ? ' · ' + r.window : ''}`),
  },
  {
    name: 'batch',
    description:
      'Runs up to 50 steps in one call, without a round trip per step: [{tool, args}] with tools click, type, ' +
      'press_keys, scroll, drag, mouse, set_value, clipboard, focus_window, window_bounds, launch_app, ' +
      'close_window, wait_for, observe, find, read_text, ocr, app_map, run_skill (built-in skills). Every step still passes the safety policy (approval ' +
      'dialogs, blocks) and the kill switch. Stops at the first error unless stop_on_error=false. Use it for ' +
      'known sequences (e.g. clicks by refs from one observe, then wait_for).',
    inputSchema: {
      type: 'object',
      properties: {
        steps: {
          type: 'array',
          minItems: 1,
          maxItems: 50,
          items: {
            type: 'object',
            properties: { tool: { type: 'string' }, args: { type: 'object' } },
            required: ['tool'],
            additionalProperties: false,
          },
        },
        stop_on_error: { type: 'boolean', description: 'Default true' },
      },
      required: ['steps'],
      additionalProperties: false,
    },
    run: forward('batch', (r) => {
      const lines = (r.steps as any[]).map((s) => {
        if (s.skipped) return `${s.step}. ${s.tool}: skipped`;
        if (!s.ok) return `${s.step}. ${s.tool}: ERROR ${s.error.code}: ${s.error.message}`;
        const v = s.result ?? {};
        const body = typeof v.text === 'string' ? `\n${v.text}` : v.met ? ` ${v.met.join(' + ')}` : v.mode ? ` · ${v.mode}` : '';
        return `${s.step}. ${s.tool}: ok${body}`;
      });
      return `${r.ok ? 'ok' : 'stopped'} batch ${r.completed}/${r.total}\n${lines.join('\n')}`;
    }),
  },
  {
    name: 'ask_human',
    description:
      'Asks the user in a dialog on their screen. kind: confirm (yes/no), choice (2-5 options), info (ok) — these ' +
      'wait for the answer (timeout_ms, default 120 s → TIMEOUT); task (something only the user can do, e.g. sign ' +
      'in, type a password, solve a check) returns a request_id at once: then call wait_for_human. ' +
      'The answer is information for you; it never approves a blocked action.',
    inputSchema: {
      type: 'object',
      properties: {
        message: { type: 'string', minLength: 1, maxLength: 1000 },
        kind: { type: 'string', enum: ['confirm', 'choice', 'info', 'task'], description: 'Default confirm' },
        options: { type: 'array', items: { type: 'string', maxLength: 40 }, minItems: 2, maxItems: 5, description: 'choice only' },
        timeout_ms: { type: 'integer', minimum: 1000, maximum: 600000 },
      },
      required: ['message'],
      additionalProperties: false,
    },
    run: forward('ask_human', (r) => (r.request_id ? `waiting for the user: ${r.request_id} (call wait_for_human)` : `answer: ${r.answer}`)),
  },
  {
    name: 'wait_for_human',
    description:
      'Waits until the user finishes a task from ask_human(kind: task): done or denied ("I cannot do this"). ' +
      'TIMEOUT leaves the request open; call again.',
    inputSchema: {
      type: 'object',
      properties: {
        request_id: { type: 'string', pattern: '^h[0-9]+$' },
        timeout_ms: { type: 'integer', minimum: 1000, maximum: 600000 },
      },
      required: ['request_id'],
      additionalProperties: false,
    },
    run: forward('wait_for_human', (r) => `user: ${r.status}`),
  },
  {
    name: 'app_map',
    description:
      'Where the user\'s apps and services are. op=scan (query?): installed apps (classic + Store) with launch ids ' +
      '— use them with launch_app instead of guessing names. op=set (service, surface desktop|browser, location: ' +
      'a launch id, or {browser, url, profile?}) remembers e.g. "whatsapp" or "gmail"; afterwards ' +
      'launch_app("whatsapp") opens it. op=get/list/remove. Stored encrypted for this Windows user only.',
    inputSchema: {
      type: 'object',
      properties: {
        op: { type: 'string', enum: ['scan', 'list', 'get', 'set', 'remove'] },
        query: { type: 'string' },
        service: { type: 'string', maxLength: 60 },
        surface: { type: 'string', enum: ['desktop', 'browser'] },
        location: { description: 'desktop: launch id/path (string); browser: {browser, url, profile?}' },
      },
      required: ['op'],
      additionalProperties: false,
    },
    run: forward('app_map', (r, a) =>
      r.text ?? (a.op === 'get' ? `${r.service}: ${r.surface} ${JSON.stringify(r.location)}`
        : a.op === 'set' ? `ok saved ${r.service} (${r.entries} entries)` : `ok ${r.removed ? 'removed' : 'not present'} ${r.service}`)),
  },
  {
    name: 'list_skills',
    description:
      'Lists skills: one call does a whole routine task. Built-in: files.list/open/mkdir/copy/move/rename/delete ' +
      '(through the Windows shell; never overwrites; delete goes to the Recycle Bin after the user\'s OK; ' +
      'system and XtraOS folders are protected). Plus the user\'s macro skills. app filters by prefix (e.g. "files").',
    inputSchema: { type: 'object', properties: { app: { type: 'string' } }, additionalProperties: false },
    run: forward('list_skills', (r) => r.text),
  },
  {
    name: 'run_skill',
    description:
      'Runs a skill by name with its args (see list_skills), e.g. {name: "files.copy", args: {from: ["C:\\\\Users\\\\me\\\\a.pdf"], ' +
      'to_dir: "C:\\\\Users\\\\me\\\\Desktop\\\\Reports"}}. Also whatsapp.send {phone: "+91…", text?, file?}. ' +
      'Every action inside still passes the safety policy; sending asks the user once, showing the number and message.',
    inputSchema: {
      type: 'object',
      properties: { name: { type: 'string', minLength: 1 }, args: { type: 'object' } },
      required: ['name'],
      additionalProperties: false,
    },
    run: forward('run_skill', (r) => {
      if (typeof r.text === 'string') return r.text;
      if (Array.isArray(r.steps)) return `${r.ok ? 'ok' : 'stopped'} skill ${r.completed}/${r.total} steps`;
      if (r.ref) return `ok opened ${r.ref} ${r.title ?? ''}`.trim();
      return 'ok';
    }),
  },
  {
    name: 'list_windows',
    description:
      'Lists open top-level windows, front first: ref, process, title, position, size, state, focused, elevated, ' +
      '`agent` = opened by the agent (only those may be closed without asking; see cleanup). ' +
      'Use the ref (e.g. "w460650") as `window` in other tools. Optional filter matches title or process.',
    inputSchema: {
      type: 'object',
      properties: { filter: { type: 'string', description: 'Case-insensitive substring of title or process name' } },
      additionalProperties: false,
    },
    run: forward('list_windows', (r) => {
      const ws = r.windows as Win[];
      return ws.length ? `${ws.length} window(s)\n` + ws.map(winLine).join('\n') : '0 windows';
    }),
  },
  {
    name: 'focus_window',
    description: 'Brings a window to the front (restores it if minimised). NEEDS_HUMAN if Windows refuses.',
    inputSchema: { type: 'object', properties: { window: WINDOW_REQ }, required: ['window'], additionalProperties: false },
    run: forward('focus_window', (r) => `ok focused ${r.ref}`),
  },
  {
    name: 'launch_app',
    description:
      'Starts an app and returns its NEW window ref. app = name ("notepad", "chrome", "firefox"), full path, or AUMID. ' +
      'If the agent already opened the same app + args and that window is still open, it is reused (focused, `reused`) ' +
      'instead of opening a duplicate; reuse=false forces a new window. ' +
      'elevated=true asks the user via the Windows admin (UAC) prompt; admin windows cannot be controlled (ELEVATION_REQUIRED).',
    inputSchema: {
      type: 'object',
      properties: {
        app: { type: 'string' },
        args: { anyOf: [{ type: 'string' }, { type: 'array', items: { type: 'string' } }], description: 'Command-line arguments' },
        elevated: { type: 'boolean' },
        reuse: { type: 'boolean', description: 'Reuse a still-open window the agent opened the same way (default true)' },
        timeout_ms: { type: 'integer', minimum: 0, maximum: 60000, description: 'Wait for the window (default 15000)' },
      },
      required: ['app'],
      additionalProperties: false,
    },
    run: forward('launch_app', (r) => `ok ${r.reused ? 'reused' : 'launched'} ${winLine(r as Win)}`),
  },
  {
    name: 'close_window',
    description:
      'Asks a window to close (like clicking X; the app may ask to save → NEEDS_HUMAN). Never force-kills. ' +
      'Closing a window the agent did not open asks the user first (close_user_window).',
    inputSchema: {
      type: 'object',
      properties: { window: WINDOW_REQ, timeout_ms: { type: 'integer', minimum: 0, maximum: 60000 } },
      required: ['window'],
      additionalProperties: false,
    },
    run: forward('close_window', (r) => `ok closed ${r.ref}`),
  },
  {
    name: 'audit_export',
    description:
      'Exports the XtraOS action log (what the agent did; no secrets) for a date range as CSV (default) or JSON, with the ' +
      'log chain check and the head hash. from/to: YYYY-MM-DD (local, inclusive). out: file path (default your ' +
      'Documents folder, xtraos-audit-<from>_<to>.csv); never overwrites; protected folders are refused.',
    inputSchema: {
      type: 'object',
      properties: {
        from: { type: 'string', pattern: '^[0-9]{4}-[0-9]{2}-[0-9]{2}$' },
        to: { type: 'string', pattern: '^[0-9]{4}-[0-9]{2}-[0-9]{2}$' },
        format: { type: 'string', enum: ['csv', 'json'] },
        out: { type: 'string' },
      },
      additionalProperties: false,
    },
    run: forward('audit_export', (r) => r.text),
  },
  {
    name: 'sandbox',
    description:
      'A separate, disposable Windows desktop (Windows Sandbox) where the agent works in parallel with the user, ' +
      'without touching the user’s cursor or keyboard. op=start (network: false by default; the first start can take ' +
      'minutes, start again keeps waiting; NEEDS_HUMAN if Windows Sandbox is not enabled or the user has their own ' +
      'Sandbox open), call {tool, args} runs any XtraOS tool inside it (results carry via: "sandbox"), stop discards ' +
      'everything, status. Inside, enter text with set_value and press buttons with click on a ref (UI Automation): ' +
      'while the Sandbox window is in the background Windows does not let a window come to the front, so type/press_keys ' +
      'return NEEDS_HUMAN. Built-in apps only (no Notepad or Store apps). Nothing from the user’s profile is inside.',
    inputSchema: {
      type: 'object',
      properties: {
        op: { type: 'string', enum: ['status', 'start', 'call', 'stop'] },
        network: { type: 'boolean' },
        tool: { type: 'string' },
        args: { type: 'object' },
        timeout_ms: { type: 'integer', minimum: 1000, maximum: 600000 },
      },
      required: ['op'],
      additionalProperties: false,
    },
    run: forward('sandbox', (r) => (typeof r.text === 'string' ? r.text : JSON.stringify(r).slice(0, 4000))),
  },
  {
    name: 'system',
    description:
      'Common PC controls through the OS, in the background: battery (read), brightness (read; value 0-100 sets the ' +
      'built-in screen), wifi / bluetooth (read; state on|off switches: asks the user first), lock and sleep (ask the ' +
      'user first). Not offered: shutdown/restart/sign-out, firewall, antivirus, accounts, network or registry changes.',
    inputSchema: {
      type: 'object',
      properties: {
        op: { type: 'string', enum: ['battery', 'brightness', 'wifi', 'bluetooth', 'lock', 'sleep'] },
        value: { type: 'integer', minimum: 0, maximum: 100 },
        state: { type: 'string', enum: ['on', 'off'] },
      },
      required: ['op'],
      additionalProperties: false,
    },
    run: forward('system', (r) => r.text),
  },
  {
    name: 'volume',
    description:
      'Windows speaker (master) volume, exact and in the background: op=get, set (value 0-100), change (by, e.g. -10), ' +
      'mute, unmute. This is the system volume, not an app/player slider. Media keys for press_keys: VolumeUp, ' +
      'VolumeDown, VolumeMute, PlayPause, NextTrack, PrevTrack, MediaStop.',
    inputSchema: {
      type: 'object',
      properties: {
        op: { type: 'string', enum: ['get', 'set', 'change', 'mute', 'unmute'] },
        value: { type: 'integer', minimum: 0, maximum: 100 },
        by: { type: 'integer', minimum: -100, maximum: 100 },
      },
      required: ['op'],
      additionalProperties: false,
    },
    run: forward('volume', (r) => r.text),
  },
  {
    name: 'touch',
    description:
      'Real touch input (screen pixels) for touch-first apps: tap, long_press, drag (to_x/to_y), pinch (scale < 1 in, ' +
      '> 1 out; distance), swipe (to_x/to_y, fingers 1-4). Waits until the user is idle (idle_ms). 3-4 finger swipes ' +
      'are Windows gestures and ask the user first. Prefer click/scroll with refs when they work.',
    inputSchema: {
      type: 'object',
      properties: {
        action: { type: 'string', enum: ['tap', 'long_press', 'drag', 'pinch', 'swipe'] },
        x: X, y: Y,
        to_x: { type: 'integer' }, to_y: { type: 'integer' },
        fingers: { type: 'integer', minimum: 1, maximum: 4 },
        scale: { type: 'number', minimum: 0.1, maximum: 10 },
        distance: { type: 'integer', minimum: 20, maximum: 2000 },
        duration_ms: { type: 'integer', minimum: 20, maximum: 10000 },
        idle_ms: IDLE,
      },
      required: ['action', 'x', 'y'],
      additionalProperties: false,
    },
    run: forward('touch', (r) => r.text),
  },
  {
    name: 'pen',
    description:
      'Real pen input with pressure (drawing and handwriting apps): tap (x, y) or draw (points: [[x, y], [x, y, ' +
      'pressure 0-1024], ...], up to 2000; pressure default 512). Waits until the user is idle.',
    inputSchema: {
      type: 'object',
      properties: {
        action: { type: 'string', enum: ['tap', 'draw'] },
        x: { type: 'integer' }, y: { type: 'integer' },
        points: { type: 'array', items: { type: 'array', items: { type: 'integer' }, minItems: 2, maxItems: 3 }, minItems: 2, maxItems: 2000 },
        pressure: { type: 'integer', minimum: 0, maximum: 1024 },
        idle_ms: IDLE,
      },
      required: ['action'],
      additionalProperties: false,
    },
    run: forward('pen', (r) => r.text),
  },
  {
    name: 'record_session',
    description:
      'Records your successful actions as a reusable skill. op=start (name: letters/digits/-_.), then act as usual, ' +
      'then op=stop saves skills/<name>.json (overwrite: true to replace); replay with run_skill {name}. Refs, windows ' +
      'and points are saved as locators (app + window title + element role/name, window-relative points) so replay ' +
      'works after windows move; a missing window/element fails that step (never guesses). Typed text becomes an ' +
      'argument (value1/text1…) you can change at replay. Password fields are never recorded. Max 50 steps, 30 min. ' +
      'op=status / discard.',
    inputSchema: {
      type: 'object',
      properties: {
        op: { type: 'string', enum: ['start', 'stop', 'status', 'discard'] },
        name: { type: 'string', pattern: '^[A-Za-z0-9_][A-Za-z0-9_.-]{0,63}$' },
        overwrite: { type: 'boolean' },
      },
      required: ['op'],
      additionalProperties: false,
    },
    run: forward('record_session', (r) => r.text),
  },
  {
    name: 'frame_diff',
    description:
      'What changed in a window (default foreground) or screen region since the previous frame_diff on it — for ' +
      'canvas apps, games and video where observe sees nothing. First call sets a baseline. Returns changed regions ' +
      '(screen pixels, ≤ 16, largest first) and changed_pct; no pixels (use screenshot/ocr on a region for those). ' +
      'wait_ms waits until at least min_pct (default 0.5) changes. Works when the window is covered, not minimised.',
    inputSchema: {
      type: 'object',
      properties: {
        window: WINDOW_REQ,
        region: { type: 'object', properties: { x: { type: 'integer' }, y: { type: 'integer' }, w: { type: 'integer' }, h: { type: 'integer' } }, required: ['x', 'y', 'w', 'h'], additionalProperties: false },
        wait_ms: { type: 'integer', minimum: 0, maximum: 60000 },
        min_pct: { type: 'number', minimum: 0, maximum: 100 },
        reset: { type: 'boolean', description: 'Start a new baseline' },
      },
      additionalProperties: false,
    },
    run: forward('frame_diff', (r) => r.text),
  },
  {
    name: 'browser',
    description:
      'Web pages in Chrome via the XtraDevPilot extension: precise CSS-selector / text targeting where UIA is weak. ' +
      'Read: list_browsers, list_tabs, tab_info, dom (clean HTML; root = CSS selector for a part), ax_tree, wait_for ' +
      '(selector, state visible|hidden|attached|detached). Act: click (selector OR text, contains), type (selector, text, ' +
      'append), press_key (key, selector?, ctrl/shift/alt), scroll (direction, amount, to_selector), navigate / open_tab ' +
      '(http/https url), switch_browser (browser_id). tab_id targets a tab. Same safety rules as the desktop: Enter / Send ' +
      'on messaging sites asks the user, buy/pay is refused, password/OTP/card fields are refused. Not available: scripts, ' +
      'cookies, storage, file upload. NEEDS_HUMAN when the XtraDevPilot bridge or extension is not running.',
    inputSchema: {
      type: 'object',
      properties: {
        action: { type: 'string', enum: ['list_browsers', 'list_tabs', 'tab_info', 'dom', 'ax_tree', 'wait_for', 'click', 'type', 'press_key', 'scroll', 'navigate', 'open_tab', 'switch_browser'] },
        selector: { type: 'string', description: 'CSS selector (XtraDevPilot also accepts text=…, text*=…, >>> chains)' },
        text: { type: 'string', description: 'click: visible text; type: text to type' },
        contains: { type: 'boolean', description: 'click by text: substring match' },
        append: { type: 'boolean' },
        key: { type: 'string', description: 'e.g. Enter, Tab, Escape, ArrowDown, a' },
        ctrl: { type: 'boolean' }, shift: { type: 'boolean' }, alt: { type: 'boolean' },
        root: { type: 'string', description: 'dom: CSS selector of the part to return' },
        state: { type: 'string', enum: ['visible', 'hidden', 'attached', 'detached'] },
        timeout_ms: { type: 'integer', minimum: 0, maximum: 55000 },
        direction: { type: 'string', enum: ['up', 'down', 'left', 'right', 'top', 'bottom'] },
        amount: { type: 'integer' },
        to_selector: { type: 'string' },
        url: { type: 'string', description: 'http:// or https:// only' },
        background: { type: 'boolean' },
        tab_id: { type: 'integer' },
        browser_id: { type: 'integer' },
      },
      required: ['action'],
      additionalProperties: false,
    },
    run: forward('browser', (r) => r.text),
  },
  {
    name: 'elevated_helper',
    description:
      'Admin windows (e.g. "Administrator: Command Prompt") cannot be controlled by normal programs; tools on them return ' +
      'ELEVATION_REQUIRED. op=start asks the user (an XtraOS dialog, then the Windows administrator prompt) to start the ' +
      'XtraOS admin helper for this session; afterwards observe/find/read_text/click/type/press_keys/... on admin windows ' +
      'work as usual (results carry via: "admin_helper"), and the helper asks the user before the first action on each ' +
      'admin window. If the user declines: NEEDS_HUMAN, do not retry without asking. op=stop ends it (also ends with the ' +
      'kill switch or after 15 idle minutes); op=status.',
    inputSchema: {
      type: 'object',
      properties: { op: { type: 'string', enum: ['start', 'stop', 'status'] } },
      required: ['op'],
      additionalProperties: false,
    },
    run: forward('elevated_helper', (r) => r.text),
  },
  {
    name: 'cleanup',
    description:
      'Closes every still-open window the agent opened in this session (only those, never windows the user opened), e.g. ' +
      'when a task is done. Confirms a browser "Leave page" prompt only in those windows. Reports what it closed and what is left.',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
    run: forward('cleanup', (r) => r.text),
  },
  {
    name: 'window_bounds',
    description:
      'Gets a window position/size/state, or sets x/y/w/h (visible frame, physical pixels) and/or state ' +
      '(minimize, maximize, restore).',
    inputSchema: {
      type: 'object',
      properties: {
        window: WINDOW_REQ,
        x: X, y: Y,
        w: { type: 'integer', minimum: 50 }, h: { type: 'integer', minimum: 50 },
        state: { type: 'string', enum: ['minimize', 'maximize', 'restore'] },
      },
      required: ['window'],
      additionalProperties: false,
    },
    run: forward('window_bounds', (r) => `${r.ref} ${r.x},${r.y} ${r.w}x${r.h} ${r.state}`),
  },
  {
    name: 'clipboard',
    description: 'Gets or sets the clipboard text.',
    inputSchema: {
      type: 'object',
      properties: {
        op: { type: 'string', enum: ['get', 'set'] },
        text: { type: 'string', description: 'Text to set (op = set)' },
      },
      required: ['op'],
      additionalProperties: false,
    },
    run: forward('clipboard', (r, a) => (a.op === 'get' ? (r.text ?? '(no text on clipboard)') : `ok clipboard set (${r.chars} chars)`)),
  },
];
