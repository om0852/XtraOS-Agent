// The MCP tool surface. MUST match docs/04-mcp-tool-contract.md (a contract test enforces it).
// M1: status. M2 (ADR 0010): click, type, press_keys, scroll, drag, mouse, clipboard.
// Coordinates are physical screen pixels; `window` is a native window handle (integer).
import type { CoreClient } from './core-client.ts';

type Schema = { type: 'object'; properties: Record<string, unknown>; required?: string[]; additionalProperties: false };

export type ToolDef = {
  name: string;
  description: string;
  inputSchema: Schema;
  run: (core: CoreClient, args: Record<string, unknown>) => Promise<string>;
};

const int = (description: string) => ({ type: 'integer', description });
const WINDOW = int('Native window handle. Mouse: optional guard (point must be inside it). Keyboard: required and must be the foreground window.');
const X = int('Screen X in physical pixels');
const Y = int('Screen Y in physical pixels');
const BUTTON = { type: 'string', enum: ['left', 'right', 'middle'], description: 'Mouse button (default left)' };

type Status = { version: string; session: string; client: string; state: string; uptime_ms: number };

/** Forwards to the core and renders a compact one-line result (docs/06: <= 30 tokens). */
const forward = (method: string, render: (r: any, a: Record<string, unknown>) => string) =>
  async (core: CoreClient, args: Record<string, unknown>) => render(await core.call(method, args), args);

export const TOOLS: ToolDef[] = [
  {
    name: 'status',
    description:
      'Shows whether the XtraOS core is running and paired: core version, session, and state. ' +
      'Call this first; if it fails, ask the user to start the XtraOS core.',
    inputSchema: { type: 'object', properties: {}, additionalProperties: false },
    async run(core) {
      const s = (await core.call('status')) as Status;
      return `XtraOS core ${s.version} · ${s.state} · session ${s.session} (${s.client}) · up ${Math.round(s.uptime_ms / 1000)}s`;
    },
  },
  {
    name: 'click',
    description:
      'Clicks with real mouse input at a screen point. Pass `window` so the click is refused (TARGET_OBSCURED) ' +
      'if the point is not inside that window. Aborts with USER_ACTIVE if the user is using the mouse.',
    inputSchema: {
      type: 'object',
      properties: {
        x: X, y: Y, window: WINDOW, button: BUTTON,
        clicks: { type: 'integer', minimum: 1, maximum: 3, description: '1 = click, 2 = double, 3 = triple' },
        modifiers: { type: 'array', items: { type: 'string', enum: ['ctrl', 'shift', 'alt', 'win'] }, description: 'Keys held during the click' },
      },
      required: ['x', 'y'],
      additionalProperties: false,
    },
    run: forward('click', (r) => `ok click ×${r.clicks} at (${r.at[0]},${r.at[1]})`),
  },
  {
    name: 'type',
    description:
      'Types unicode text (any language, emoji) with real keyboard input into `window`, which must be the ' +
      'foreground window (else NEEDS_HUMAN, nothing typed). Newline = Enter, tab = Tab. The text is never logged.',
    inputSchema: {
      type: 'object',
      properties: {
        window: WINDOW,
        text: { type: 'string', maxLength: 10000, description: 'Text to type' },
        submit: { type: 'boolean', description: 'Press Enter afterwards' },
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
      },
      required: ['window', 'keys'],
      additionalProperties: false,
    },
    run: forward('press_keys', (r) => `ok ${r.chords} chord(s)`),
  },
  {
    name: 'scroll',
    description: 'Scrolls the mouse wheel at a point. dy > 0 scrolls down, dx > 0 scrolls right (notches).',
    inputSchema: {
      type: 'object',
      properties: {
        x: X, y: Y, window: WINDOW,
        dy: { type: 'integer', minimum: -50, maximum: 50 },
        dx: { type: 'integer', minimum: -50, maximum: 50 },
      },
      required: ['x', 'y'],
      additionalProperties: false,
    },
    run: forward('scroll', (r) => `ok scroll dy=${r.dy} dx=${r.dx}`),
  },
  {
    name: 'drag',
    description:
      'Real drag-and-drop: press at (from_x, from_y), move in steps, release at (to_x, to_y). Works with OS ' +
      'drag-and-drop. `window` guards the start point, `to_window` the drop point. Released if anything fails.',
    inputSchema: {
      type: 'object',
      properties: {
        from_x: X, from_y: Y, to_x: X, to_y: Y, window: WINDOW, to_window: WINDOW, button: BUTTON,
        steps: { type: 'integer', minimum: 1, maximum: 500, description: 'Intermediate moves (default 20)' },
        step_ms: { type: 'integer', minimum: 0, maximum: 200, description: 'Delay per step (default 12)' },
        hold_ms: { type: 'integer', minimum: 0, maximum: 2000, description: 'Pause after press and before release (default 60)' },
      },
      required: ['from_x', 'from_y', 'to_x', 'to_y'],
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
      },
      required: ['action'],
      additionalProperties: false,
    },
    run: forward('mouse', (r, a) => (a.action === 'path' ? `ok path ${r.points} points` : `ok ${a.action} at (${r.at[0]},${r.at[1]})`)),
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
