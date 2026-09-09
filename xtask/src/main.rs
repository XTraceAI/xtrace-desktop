//! Workspace maintenance entry point. Implemented tasks are listed in help.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty()
        || matches!(args.as_slice(), [arg] if matches!(arg.as_str(), "help" | "-h" | "--help"))
    {
        println!(
            "XTrace workspace tasks\n\nUsage: cargo xtask [help]\n\nOnly help is available in the foundation scaffold."
        );
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "Unsupported task: {}\nRun `cargo xtask help` for available tasks.",
            args.join(" ")
        );
        ExitCode::from(2)
    }
}
