//! XtraOS Agent core runtime. M1: secure local IPC, pairing, sessions, action log.
//! M2: real input (mouse, keyboard, wheel, drag, clipboard) with safety guards.
//! M3: windows & apps (list, focus, launch, close, bounds) + elevation boundary.
//! M4: perception (UIA tree, refs, diffs, screenshots) + background-first actions.
//! M5: safety: kill switch, policy engine, human-in-the-loop dialogs, overlay.
//! P2-M5: elevated admin helper (helper.rs, ADR 0030).
//! P2-M6: browser bridge client to XtraDevPilot (browser.rs, ADR 0032).
//! Only this crate talks to the OS (CLAUDE.md §3).
pub mod actionlog;
pub mod appmap;
pub mod browser;
pub mod capture;
pub mod exec;
pub mod helper;
pub mod input;
pub mod ipc;
pub mod ocr;
pub mod pairing;
pub mod perceive;
pub mod policy;
pub mod protocol;
pub mod safety;
pub mod skills;
pub mod winmgr;
pub mod winsec;
