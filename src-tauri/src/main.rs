#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = shikigami_lib::run_approval_relay(std::env::args_os()) {
        std::process::exit(code);
    }
    shikigami_lib::run()
}
