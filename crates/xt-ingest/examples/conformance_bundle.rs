//! Test-only bridge to the same bundle verification the app uses. No producer runs here.
use std::path::Path;
use xt_ingest::native::readers_cli::{read_pin, verify_bundle};

fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        eprintln!("Expected pin file and bundle root");
        std::process::exit(1);
    }
    let result =
        read_pin(Path::new(&args[0])).and_then(|pin| verify_bundle(&pin, Path::new(&args[1])));
    match result {
        Ok(producer) => println!("{}", producer.commit),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}
