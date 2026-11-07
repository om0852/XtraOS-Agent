# Recorded flows (Release 0.1)

P1-REL-02: each flow drives the release bundle through MCP exactly as an agent would. Re-record: `node tests/flows/run-flows.mjs`.

| Flow | What | Result | Calls | ~Result tokens | Time in tools |
|---|---|---|---|---|---|
| [F2](F2.md) | Observe → act loop (Calculator, background clicks) | PASS | 8 | 341 | 1.3 s |
| [F3](F3.md) | Chrome search → close → Firefox (no extension) search → close | PASS | 10 | 161 | 2.4 s |
| [F4](F4.md) | Drag a file from Explorer onto a web page drop zone (refs) | PASS | 13 | 369 | 3.3 s |
| [F6](F6.md) | Terminal: run a command and read its output | PASS | 5 | 222 | 1.2 s |
| [F10](F10.md) | Kill switch during a long drag, then the user resumes | PASS | 8 | 163 | 2.4 s |
| [F8](F8.md) | Opaque (hand-drawn) window: observe finds nothing, OCR finds and clicks the text | PASS | 6 | 127 | 1.4 s |
| [S1](S1.md) | Routine file task in ONE call: macro skill (new folder + copy + open) | PASS | 4 | 43 | 2.1 s |
| [R1](R1.md) | Routine task in ≤ 3 calls: Calculator 7 + 5 with wait_for + batch (P2-BENCH-01) | PASS | 3 | 335 | 2.5 s |
