//! Minimal Windows-only injector: finds the game process and injects
//! hook.dll next to this executable. No GUI, no WebView2 — extracted from
//! `check_and_perform_hook` in the Tauri app's `main.rs` so the injection
//! step can run under Wine without ever touching DirectComposition.
//!
//! Uses plain println!/eprintln! rather than the `log` crate: this process's
//! whole purpose is to have its stdout/stderr piped and displayed elsewhere
//! (the TUI's log panel), so a logging framework with its own level filtering
//! (silently dropped unless RUST_LOG is set) just adds a footgun for no
//! benefit here. stdout is block-buffered rather than line-buffered when
//! piped (i.e. not a terminal), so every print is followed by an explicit
//! flush — otherwise these messages can sit in the buffer indefinitely since
//! this process runs in a loop and rarely writes enough to fill it.

use std::io::{self, Write};
use std::path::Path;
use std::time::Duration;

use dll_syringe::{process::OwnedProcess, Syringe};

const GAME_PROCESS_NAME: &str = "granblue_fantasy_relink.exe";
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

fn log(message: impl std::fmt::Display) {
    println!("{message}");
    let _ = io::stdout().flush();
}

fn main() {
    log(format!("injector starting, watching for {GAME_PROCESS_NAME}"));

    loop {
        match OwnedProcess::find_first_by_name(GAME_PROCESS_NAME) {
            Some(target) => {
                let syringe = Syringe::for_process(target);
                let dll_path = Path::new("hook.dll");

                log(format!("found game process, injecting DLL: {:?}", dll_path));

                match syringe.inject(dll_path) {
                    Ok(_) => log("injected hook.dll successfully"),
                    Err(e) => log(format!("failed to inject hook.dll: {e}")),
                }

                // Wait for the game to exit before looking for it again, so we
                // don't try to inject twice into the same process.
                wait_for_exit();
            }
            None => {
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

fn wait_for_exit() {
    while OwnedProcess::find_first_by_name(GAME_PROCESS_NAME).is_some() {
        std::thread::sleep(POLL_INTERVAL);
    }
    log("game process has exited, watching for it to restart.");
}
