//! XtraOS Agent core runtime. M1: secure local IPC, pairing, sessions, action log.
//! M2: real input (mouse, keyboard, wheel, drag, clipboard) with safety guards.
//! M3: windows & apps (list, focus, launch, close, bounds) + elevation boundary.
//! Only this crate talks to the OS (CLAUDE.md §3).
pub mod actionlog;
pub mod input;
pub mod ipc;
pub mod pairing;
pub mod protocol;
pub mod winmgr;
pub mod winsec;
