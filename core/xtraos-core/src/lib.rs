//! XtraOS Agent core runtime. Phase 1 M1: secure local IPC, pairing, sessions, action log.
//! Only this crate talks to the OS (CLAUDE.md §3).
pub mod actionlog;
pub mod ipc;
pub mod pairing;
pub mod protocol;
pub mod winsec;
