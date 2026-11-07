// The MCP tool surface. MUST match docs/04-mcp-tool-contract.md (a contract test enforces it).
// M1: status. M2 (ADR 0010): click, type, press_keys, scroll, drag, mouse, clipboard.
// M3 (ADR 0012): list_windows, focus_window, launch_app, close_window, window_bounds.
// M4 (ADRs 0014, 0015): observe, find, read_text, screenshot, set_value; `ref` targets act in the
// background first (no cursor, no keys); real input waits for the user to be idle.
// M5 (ADR 0017): ask_human, wait_for_human; every action passes the core's policy engine and
// kill switch (BLOCKED_BY_POLICY, STOPPED — only the user resumes, from the tray).
// P2-M1 (ADR 0022): wait_for, batch, observe(scope_ref) — fewer agent round trips.
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
  description: 'Window ref from list_windows (e.g. "w460650") or its integer handle. Mouse: optional guard (point must be inside it). Keyboard: required and must be the foreground window.',
};
const WINDOW_REQ = { type: ['string', 'integer'], description: 'Window ref from list_windows (e.g. "w460650") or its integer handle' };

type Win = { ref: string; process: string; title: string; x: number; y: number; w: number; h: number; state: string; focused: boolean; elevated: boolean };
/** One compact line per window (docs/06 token budget). */
const winLine = (w: Win) =>
  `${w.ref} ${w.process} "${w.title.length > 80 ? w.title.slice(0, 79) + '…' : w.title}" ${w.x},${w.y} ${w.w}x${w.h}` +
  `${w.state !== 'normal' ? ' ' + w.state : ''}${w.focused ? ' focused' : ''}${w.elevated ? ' elevated' : ''}`;
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

type Status = { version: string; session: string; client: string; state: string; uptime_ms: number; kill_hotkey: string | null };

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
        (s.kill_hotkey ? ` · stop: ${s.kill_hotkey}` : ' · stop: tray icon');
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
      'including ones scrolled out of view; returns up to 20 refs.',
    inputSchema: {
      type: 'object',
      properties: { query: { type: 'string', minLength: 1 }, window: WINDOW_REQ },
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
      'close_window, wait_for, observe, find, read_text. Every step still passes the safety policy (approval ' +
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
    name: 'list_windows',
    description:
      'Lists open top-level windows, front first: ref, process, title, position, size, state, focused, elevated. ' +
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
      'elevated=true asks the user via the Windows admin (UAC) prompt; admin windows cannot be controlled (ELEVATION_REQUIRED).',
    inputSchema: {
      type: 'object',
      properties: {
        app: { type: 'string' },
        args: { anyOf: [{ type: 'string' }, { type: 'array', items: { type: 'string' } }], description: 'Command-line arguments' },
        elevated: { type: 'boolean' },
        timeout_ms: { type: 'integer', minimum: 0, maximum: 60000, description: 'Wait for the window (default 15000)' },
      },
      required: ['app'],
      additionalProperties: false,
    },
    run: forward('launch_app', (r) => `ok launched ${winLine(r as Win)}`),
  },
  {
    name: 'close_window',
    description: 'Asks a window to close (like clicking X; the app may ask to save → NEEDS_HUMAN). Never force-kills.',
    inputSchema: {
      type: 'object',
      properties: { window: WINDOW_REQ, timeout_ms: { type: 'integer', minimum: 0, maximum: 60000 } },
      required: ['window'],
      additionalProperties: false,
    },
    run: forward('close_window', (r) => `ok closed ${r.ref}`),
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
