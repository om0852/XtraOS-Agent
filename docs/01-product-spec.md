# 01 — Product Spec

## One line

**XtraOS Agent is a local AI action runtime: a bridge that lets any AI model see and operate a
computer — apps, browsers, files, games — safely, quickly, and with as few tokens as possible.**

The LLM is the brain. XtraOS Agent is the hands, the eyes, and the seatbelt.

## Category

An **Agent–Computer Interface (ACI) platform**, delivered as:

| Deliverable | What it is |
|---|---|
| `xtraos-core` | Native background runtime (Rust). Owns input, perception, app map, policy, log. |
| `xtraos-mcp` | MCP server (TypeScript). The interface AI clients connect to. |
| `xtraos-tray` | Desktop app (Tauri): kill switch, overlay, approvals, settings, activity log. |
| Drivers | Desktop driver (built-in), Browser driver (= XtraDevPilot extension), later Mobile. |
| SDK (later) | Node/Python libraries for scripting without an LLM. |

## Who it is for

1. **Power users / creators** who want an AI to do day-to-day computer tasks.
2. **Developers** who want their AI coding agent to also drive apps, terminals, and browsers.
3. **Teams** (later) who need audited, policy-controlled automation.

## Core promises

1. **Do anything a human can do with mouse and keyboard** on a normal desktop session.
2. **Human in the loop** for anything privileged or sensitive (admin, sending, deleting, paying).
3. **Cheap and fast**: accessibility-tree perception and skills keep tokens and round trips low.
4. **Local and private**: no cloud dependency, no LLM inside, nothing leaves the machine
   except what the AI client itself sends to its model.

## In scope (by phase — see 07-development-plan.md)

- Phase 1 (Foundation, Windows): input (click, double/right click, drag & drop, scroll, type,
  key chords), window/app control (list, launch, focus, close), perception (UI Automation tree,
  screenshots), human-in-the-loop requests, kill switch + overlay, action log, local pairing.
- Phase 2 (Smart): element references + diffs, local OCR fallback, app map ("where is my
  WhatsApp"), skills (multi-step actions in one call), trusted batches, browser driver bridge
  to XtraDevPilot, elevated helper (opt-in).
- Phase 3 (Reach): games/canvas helpers, touch & gestures, macOS, recording/replay.
- Phase 4 (Platform): Linux, SDK, team policies, skill marketplace.

## Explicit non-goals (never)

- CAPTCHA solving, bot-detection evasion, "undetectable" automation.
- Automating UAC consent, the lock screen, or OS credential prompts.
- Typing passwords/payment data on behalf of the user.
- Remote control over the network, hidden/background operation without the overlay.
- Running an LLM inside the product.

## Example jobs (acceptance stories)

| Story | Phase |
|---|---|
| Open Chrome, search a term, close it, open Firefox (no extension), search again. | 1 |
| Open Notepad, type a note, save it to the Desktop. | 1 |
| Drag a file from Explorer onto a browser upload zone. | 1 |
| Open CMD as admin → human approves UAC → run `ipconfig` (needs elevated helper). | 2 |
| Open Claude Code in a terminal, type a prompt, press Enter, read the result. | 1 (terminal text) / 2 (smart) |
| "Send report.pdf to Rahul on WhatsApp" using the app map + confirmation. | 2 |
| Play a simple canvas/browser game via screenshots + real input. | 3 |

## Success metrics

- Routine task (e.g. send a file via a known app) in **≤ 3 AI tool calls** and **≤ 2,000 tokens**.
- Observation (UI tree of the focused window) **≤ 400 tokens median**.
- Action latency (click by element ref) **≤ 80 ms p50** on a mid-range PC.
- Kill switch stops all input in **≤ 100 ms**.
- Zero actions executed outside policy (audited by the action log).
