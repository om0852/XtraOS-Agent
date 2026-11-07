# Recorded flows (Release 0.1)

P1-REL-02: each flow drives the release bundle through MCP exactly as an agent would. Re-record: `node tests/flows/run-flows.mjs`.

| Flow | What | Result | Calls | ~Result tokens | Time in tools |
|---|---|---|---|---|---|
| [F2](F2.md) | Observe → act loop (Calculator, background clicks) | PASS | 8 | 341 | 2.2 s |
| [F3](F3.md) | Chrome search → close → Firefox (no extension) search → close | PASS | 13 | 168 | 3.0 s |
| [F4](F4.md) | Drag a file from Explorer onto a web page drop zone (refs) | PASS | 13 | 321 | 5.3 s |
| [F6](F6.md) | Terminal: run a command and read its output | PASS | 5 | 222 | 1.7 s |
| [F10](F10.md) | Kill switch during a long drag, then the user resumes | PASS | 8 | 162 | 3.7 s |
