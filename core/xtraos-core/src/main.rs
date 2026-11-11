//! `xtraos-core` binary: starts the pipe server and runs until Ctrl+C (or the session ends).
//! Environment overrides (tests): XTRAOS_PIPE, XTRAOS_TOKEN, XTRAOS_HOME.
//! Release builds are windowless (no console window at login or when MCP starts the core,
//! ADR 0019); output still reaches redirected stdout/stderr (tests read the READY line).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::sync::Arc;

use xtraos_core::{actionlog::ActionLog, ipc, pairing, protocol};

fn main() {
    // `--elevated-helper …`: this process is the admin helper (ADR 0030), not the core.
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--elevated-helper") {
        std::process::exit(xtraos_core::helper::helper_main(&args));
    }
    // `--sandbox-agent <exchange>`: the XtraOS inside Windows Sandbox (ADR 0040).
    if args.first().map(String::as_str) == Some("--sandbox-agent") {
        std::process::exit(xtraos_core::sandbox::agent_main(&args));
    }
    // `call <tool> [params]`: command-line bridge for any LLM / agent (ADR 0039).
    if args.first().map(String::as_str) == Some("call") {
        std::process::exit(xtraos_core::cli::call_main(&args));
    }
    // `--verify-log`: check the action log's hash chain (ADR 0038); exit 0 = intact.
    if args.first().map(String::as_str) == Some("--verify-log") {
        xtraos_core::cli::attach_console();
        let dir = pairing::data_dir().join("logs");
        let v = xtraos_core::actionlog::verify_dir(&dir);
        println!("{} ({})", v.message, dir.display());
        std::process::exit(if v.ok { 0 } else { 1 });
    }
    // `--viewer`: the read-only rules and activity window (ADR 0034).
    if args.first().map(String::as_str) == Some("--viewer") {
        std::process::exit(xtraos_core::viewer::run_viewer());
    }
    core_main();
}

#[tokio::main]
async fn core_main() {
    if let Err(e) = run().await {
        eprintln!("[xtraos-core] fatal: {e}");
        std::process::exit(1);
    }
}

async fn run() -> std::io::Result<()> {
    let dir = pairing::data_dir();
    let pipe = pairing::pipe_name()?;
    let token = pairing::load_or_create_token(&dir)?;
    let log = Arc::new(ActionLog::create(&dir.join("logs"))?);

    let server = ipc::start(ipc::ServerConfig {
        pipe_name: pipe.clone(),
        token,
        log: log.clone(),
    })
    .map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("could not create pipe {pipe} (already running, or name taken?): {e}"),
        )
    })?;
    pairing::write_connection_info(&dir, &pipe)?;
    // P7-M4 (ADR 0063): scheduled routines run inside the core, never via Task Scheduler.
    xtraos_core::routines::start(log.clone());

    eprintln!("[xtraos-core] {} listening on {pipe}", protocol::VERSION);
    eprintln!("[xtraos-core] action log: {}", log.path().display());
    println!("READY {pipe}"); // machine-readable readiness line for tests/launchers

    tokio::signal::ctrl_c().await?;
    eprintln!("[xtraos-core] stopping");
    server.shutdown().await
}
