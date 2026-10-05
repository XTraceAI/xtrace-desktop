#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = xtrace_desktop::claude_usage_helper_exit_code() {
        std::process::exit(code);
    }
    xtrace_desktop::run();
}
