//! Replay the existing native path against a supplied local index. Intended for
//! offline acceptance copies; native source files are only read.
use std::{env, path::PathBuf};
use xt_ingest::native::{
    ImportRequest, ProducerSource, ScanMode, scan_native_continued,
    session_creation::{SpawnBacklog, continue_codex_spawns, spawn_limits},
};
use xt_store::{Host, Store};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args().collect();
    if args.len() != 9
        || args[1] != "--db"
        || args[3] != "--home"
        || args[5] != "--pin"
        || args[7] != "--producer-root"
    {
        return Err(
            "usage: human_native_replay --db INDEX --home HOME --pin PIN --producer-root PLUGIN_DIRECTORY"
                .into(),
        );
    }
    let home = PathBuf::from(&args[4]);
    xt_ingest::native::validate_index_destination(&PathBuf::from(&args[2]), &home)?;
    let producer = ProducerSource::Checkout {
        pin: PathBuf::from(&args[6]),
        plugin_root: Some(PathBuf::from(&args[8])),
    };
    let mut store = Store::open(&args[2])?;
    let mut backlog = SpawnBacklog::starting();
    let now = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    )?;
    let report = scan_native_continued(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &[Host::Codex],
            producer: &producer,
            python: None,
            observed_at: now,
            cancel: None,
        },
        ScanMode::Replay,
        &mut |_| {},
        &mut backlog,
        spawn_limits(),
    );
    let mut passes = 0;
    let mut origin_changes = 0;
    while backlog.pending() && passes < 10_000 {
        let progress =
            continue_codex_spawns(&mut store, &home, &mut backlog, spawn_limits(), None, now)?;
        origin_changes += progress.changed;
        passes += 1;
        if !progress.advanced() {
            break;
        }
    }
    // Metadata counts only: never print source paths, IDs or transcript text.
    for host in report.hosts {
        println!(
            "{}",
            serde_json::json!({"host":host.host,"status":host.status,"sessions":host.sessions.len(),"origin":host.origin,"remaining_headers":backlog.pending(),"additional_header_changes":origin_changes})
        );
    }
    Ok(())
}
