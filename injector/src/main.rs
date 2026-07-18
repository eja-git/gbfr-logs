//! Minimal Windows-only injector: finds the game process and injects
//! hook.dll next to this executable. No GUI, no WebView2 — extracted from
//! `check_and_perform_hook` in the Tauri app's `main.rs` so the injection
//! step can run under Wine without ever touching DirectComposition.

use std::path::Path;
use std::time::Duration;

use dll_syringe::{process::OwnedProcess, Syringe};

const GAME_PROCESS_NAME: &str = "granblue_fantasy_relink.exe";
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

fn main() {
    env_logger::init();

    loop {
        match OwnedProcess::find_first_by_name(GAME_PROCESS_NAME) {
            Some(target) => {
                let syringe = Syringe::for_process(target);
                let dll_path = Path::new("hook.dll");

                log::info!("Found game process, injecting DLL: {:?}", dll_path);

                match syringe.inject(dll_path) {
                    Ok(_) => log::info!("Injected hook.dll successfully"),
                    Err(e) => log::error!("Failed to inject hook.dll: {e}"),
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
    log::info!("Game process has exited, watching for it to restart.");
}
