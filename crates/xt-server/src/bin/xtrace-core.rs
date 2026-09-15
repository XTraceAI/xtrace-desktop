use std::{
    io::{self, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};
use xt_store::Store;

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("{message}");
        std::process::exit(1);
    }
}
const USAGE: &str = "Usage: xtrace-core serve --db PATH [--port PORT] [--bind 127.0.0.1|::1]\n       xtrace-core import-native --db PATH --home DIR [--host claude|codex|cursor]... [--pin FILE] [--plugin-root DIR] [--python EXE] [--replay]\n       xtrace-core watch-native --db PATH --home DIR [--host claude|codex|cursor]... [--pin FILE] [--plugin-root DIR] [--python EXE] [--once | --for SECONDS]";

async fn run() -> Result<(), &'static str> {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("serve") => {}
        Some("import-native") => return import_native(args),
        Some("watch-native") => return watch_native(args),
        _ => return Err(USAGE),
    }
    let (mut db, mut port, mut ip) = (None, None, None);
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("A startup option is missing its value")?;
        match flag.to_str() {
            Some("--db") if db.is_none() => db = Some(PathBuf::from(value)),
            Some("--port") if port.is_none() => {
                port = Some(
                    value
                        .to_str()
                        .and_then(|s| s.parse::<u16>().ok())
                        .ok_or("Invalid server port")?,
                )
            }
            Some("--bind") if ip.is_none() => {
                ip = Some(
                    value
                        .to_str()
                        .and_then(|s| s.parse::<IpAddr>().ok())
                        .ok_or("Invalid literal bind address")?,
                )
            }
            _ => return Err("Unknown or duplicate startup option"),
        }
    }
    let db = db
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("--db PATH is required")?;
    let ip = ip.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    if ip != IpAddr::V4(Ipv4Addr::LOCALHOST) && ip != IpAddr::V6(std::net::Ipv6Addr::LOCALHOST) {
        return Err("Server must bind a literal localhost address");
    }
    let store = Store::open(&db).map_err(|_| "Could not open the server database")?;
    let configured = if port.is_some() {
        None
    } else {
        store
            .server_port()
            .map_err(|_| "Invalid persisted server.port setting")?
    };
    let listener = xt_server::bind(SocketAddr::new(
        ip,
        port.or(configured).unwrap_or(xt_server::DEFAULT_PORT),
    ))
    .await
    .map_err(|_| "Could not bind the configured loopback port; it may be occupied")?;
    let address = listener
        .local_addr()
        .map_err(|_| "Could not read bound server address")?;
    println!("XTRACE_PORT={}", address.port());
    io::stdout()
        .flush()
        .map_err(|_| "Could not report bound server port")?;
    xt_server::serve(listener, store, db, async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .map_err(|_| "Loopback server failed")
}

struct NativeOptions {
    db: PathBuf,
    home: PathBuf,
    hosts: Vec<xt_store::Host>,
    pin: PathBuf,
    plugin_root: Option<PathBuf>,
    python: Option<std::ffi::OsString>,
    replay: bool,
    /// `watch-native`: exit once ready (`--once`) or after this many seconds (`--for`).
    duration: Option<Option<u64>>,
}

fn parse_native_options(
    mut args: std::iter::Skip<std::env::ArgsOs>,
    watch: bool,
) -> Result<NativeOptions, &'static str> {
    use xt_store::Host;
    let (mut db, mut home, mut pin, mut plugin_root, mut python) = (None, None, None, None, None);
    let mut hosts = Vec::new();
    let mut replay = false;
    let mut duration: Option<Option<u64>> = None;
    while let Some(flag) = args.next() {
        match flag.to_str() {
            Some("--replay") if !watch && !replay => {
                replay = true;
                continue;
            }
            Some("--once") if watch && duration.is_none() => {
                duration = Some(None);
                continue;
            }
            _ => {}
        }
        let value = args.next().ok_or("An import option is missing its value")?;
        match flag.to_str() {
            Some("--db") if db.is_none() => db = Some(PathBuf::from(value)),
            Some("--home") if home.is_none() => home = Some(PathBuf::from(value)),
            Some("--pin") if pin.is_none() => pin = Some(PathBuf::from(value)),
            Some("--plugin-root") if plugin_root.is_none() => {
                plugin_root = Some(PathBuf::from(value))
            }
            Some("--python") if python.is_none() => python = Some(value),
            Some("--for") if watch && duration.is_none() => {
                duration = Some(Some(
                    value
                        .to_str()
                        .and_then(|s| s.parse::<u64>().ok())
                        .ok_or("--for requires a whole number of seconds")?,
                ))
            }
            Some("--host") => {
                let host = match value.to_str() {
                    Some("claude") => Host::Claude,
                    Some("codex") => Host::Codex,
                    Some("cursor") => Host::Cursor,
                    _ => return Err("--host must be claude, codex or cursor"),
                };
                if hosts.contains(&host) {
                    return Err("Unknown or duplicate import option");
                }
                hosts.push(host);
            }
            _ => return Err("Unknown or duplicate import option"),
        }
    }
    let db = db
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or("--db PATH is required")?;
    let home = home
        .filter(|p| p.is_dir())
        .ok_or("--home DIR must name an existing directory")?;
    if hosts.is_empty() {
        hosts = vec![Host::Claude, Host::Codex, Host::Cursor];
    }
    let plugin_root =
        plugin_root.or_else(|| std::env::var_os("AGENT_PLUGINS_DIR").map(PathBuf::from));
    let pin = pin.unwrap_or_else(|| PathBuf::from(".plugin-pin"));
    Ok(NativeOptions {
        db,
        home,
        hosts,
        pin,
        plugin_root,
        python,
        replay,
        duration,
    })
}

fn clock_ms() -> Result<i64, &'static str> {
    i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Import clock is unavailable")?
            .as_millis(),
    )
    .map_err(|_| "Import clock is unavailable")
}

/// One-shot import of native session history through the pinned shared readers,
/// resuming behind proven checkpoints unless `--replay` is given. Prints a JSON
/// report; exits 0 when every requested host imported completely, 2 when a host
/// or session was unavailable, unreadable or incomplete.
fn import_native(args: std::iter::Skip<std::env::ArgsOs>) -> Result<(), &'static str> {
    use xt_ingest::native::{ImportRequest, ScanMode, scan_native};
    let options = parse_native_options(args, false)?;
    let observed_at = clock_ms()?;
    validate_index_destination(&options.db, &options.home)?;
    let mut store = Store::open(&options.db).map_err(|_| "Could not open the index database")?;
    let report = scan_native(
        &mut store,
        &ImportRequest {
            home: &options.home,
            hosts: &options.hosts,
            pin: &options.pin,
            plugin_root: options.plugin_root.as_deref(),
            python: options.python.as_deref(),
            observed_at,
        },
        if options.replay {
            ScanMode::Replay
        } else {
            ScanMode::Resume
        },
    );
    let text =
        serde_json::to_string_pretty(&report).map_err(|_| "Could not encode the import report")?;
    println!("{text}");
    io::stdout()
        .flush()
        .map_err(|_| "Could not write the import report")?;
    if !report.complete() {
        std::process::exit(2);
    }
    Ok(())
}

/// Watch native history and keep the index current: the watcher registers
/// before the initial scan, changes queued meanwhile are reconciled before
/// `ready`, then each burst of changes is reconciled live. Prints one JSON
/// line per event. `--once` exits after `ready`; `--for` exits after the given
/// seconds; otherwise it runs until interrupted. Both bounds cover the startup
/// too: `--for` elapsing or Ctrl-C arriving during the initial scan stops the
/// tailer once that scan is done, reconciling what it received, and the run
/// exits 1 as not ready. Exits 0 when the final freshness is live and every
/// scan was complete, 2 otherwise. An event that cannot be written (the
/// consumer of the stream is gone) stops the tailer and fails the run, so the
/// watcher never runs on with no observable output.
fn watch_native(args: std::iter::Skip<std::env::ArgsOs>) -> Result<(), &'static str> {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };
    use std::time::Duration;
    use xt_ingest::native::watch::{Freshness, TailEvent, Tailer, WatchConfig};
    let options = parse_native_options(args, true)?;
    validate_index_destination(&options.db, &options.home)?;
    let store = Store::open(&options.db).map_err(|_| "Could not open the index database")?;
    let complete = Arc::new(AtomicBool::new(true));
    let live = Arc::new(AtomicBool::new(false));
    let output = Arc::new(Mutex::new(io::stdout()));
    let output_failed = Arc::new(AtomicBool::new(false));
    // Ends the wait below: Ctrl-C, or an event that could not be written.
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let sink = {
        let (complete, live, output, output_failed, stop) = (
            Arc::clone(&complete),
            Arc::clone(&live),
            Arc::clone(&output),
            Arc::clone(&output_failed),
            stop_tx.clone(),
        );
        Box::new(move |event: TailEvent| {
            // Freshness is taken from every event, so a root that could not
            // be watched after readiness still fails the run.
            match &event {
                TailEvent::Ready(readiness) => {
                    live.store(readiness.freshness == Freshness::Live, Ordering::SeqCst);
                    if !readiness.report.complete() {
                        complete.store(false, Ordering::SeqCst);
                    }
                }
                TailEvent::Reconciled {
                    freshness, report, ..
                } => {
                    live.store(*freshness == Freshness::Live, Ordering::SeqCst);
                    if !report.complete() {
                        complete.store(false, Ordering::SeqCst);
                    }
                }
                TailEvent::Stopped { freshness } => {
                    live.store(*freshness == Freshness::Live, Ordering::SeqCst);
                }
            }
            let written = serde_json::to_string(&event)
                .ok()
                .and_then(|text| {
                    let mut out = output.lock().ok()?;
                    writeln!(out, "{text}").ok()?;
                    out.flush().ok()
                })
                .is_some();
            if !written {
                output_failed.store(true, Ordering::SeqCst);
                let _ = stop.send(());
            }
        }) as Box<dyn Fn(TailEvent) + Send>
    };
    // The bound on the run is armed before the tailer starts, so it covers
    // the startup: `--for` elapsing or Ctrl-C arriving during the initial
    // scan stops the tailer once that scan is done instead of running on or
    // killing the process with changes received and unreconciled.
    match options.duration {
        Some(None) => {}
        Some(Some(seconds)) => {
            let stop = stop_tx.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(seconds));
                let _ = stop.send(());
            });
        }
        None => ctrlc_wait(stop_tx.clone()),
    }
    let tailer = Tailer::start(
        store,
        WatchConfig {
            home: options.home.clone(),
            hosts: options.hosts.clone(),
            pin: options.pin.clone(),
            plugin_root: options.plugin_root.clone(),
            python: options.python.clone(),
            debounce: Duration::from_millis(250),
            probe: None,
        },
        sink,
    );
    // Readiness counts only if no stop was queued first: a bound reached
    // while the startup was still running, even within the same wait step,
    // means the run did not become ready within its bound.
    let ready = loop {
        let ready = tailer.wait_ready(Duration::from_millis(200));
        let stop_requested = match stop_rx.try_recv() {
            Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => true,
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
        };
        if stop_requested || tailer.status().stopped {
            break None;
        }
        if ready.is_some() {
            break ready;
        }
    };
    if ready.is_none() {
        tailer.stop();
        if output_failed.load(Ordering::SeqCst) {
            return Err("Could not write the event stream");
        }
        return Err("Native watcher stopped before it was ready");
    }
    if options.duration != Some(None) {
        let _ = stop_rx.recv();
    }
    tailer.stop();
    if output_failed.load(Ordering::SeqCst) {
        return Err("Could not write the event stream");
    }
    if !(live.load(Ordering::SeqCst) && complete.load(Ordering::SeqCst)) {
        std::process::exit(2);
    }
    Ok(())
}

/// Resolve the sender when Ctrl-C arrives (or the runtime cannot listen).
fn ctrlc_wait(stop: std::sync::mpsc::Sender<()>) {
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = stop.send(());
    });
}

/// Validate before SQLite can create tables or sidecars. Existing hardlinks are
/// rejected because an alternate name can otherwise alias native history.
fn validate_index_destination(
    db: &std::path::Path,
    home: &std::path::Path,
) -> Result<(), &'static str> {
    fn resolved(path: &std::path::Path) -> std::io::Result<PathBuf> {
        match path.canonicalize() {
            Ok(path) => Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if std::fs::symlink_metadata(path).is_ok() {
                    return Err(error);
                }
                let parent = path
                    .parent()
                    .filter(|p| !p.as_os_str().is_empty())
                    .unwrap_or(std::path::Path::new("."));
                Ok(resolved(parent)?.join(path.file_name().ok_or(error)?))
            }
            Err(error) => Err(error),
        }
    }
    fn reject_reverse_aliases(
        root: &std::path::Path,
        destinations: &[PathBuf],
    ) -> Result<(), &'static str> {
        let metadata = match std::fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err("Cannot inspect native source aliases"),
        };
        if metadata.file_type().is_symlink() {
            let target =
                std::fs::read_link(root).map_err(|_| "Cannot inspect native source alias")?;
            let target = resolved(
                &root
                    .parent()
                    .unwrap_or(std::path::Path::new("."))
                    .join(target),
            )
            .map_err(|_| "Cannot resolve native source alias")?;
            if destinations.iter().any(|path| path.starts_with(&target)) {
                return Err("Native source alias points at index destination");
            }
        } else if metadata.is_dir() {
            for entry in
                std::fs::read_dir(root).map_err(|_| "Cannot inspect native source aliases")?
            {
                reject_reverse_aliases(
                    &entry
                        .map_err(|_| "Cannot inspect native source entry")?
                        .path(),
                    destinations,
                )?;
            }
        }
        Ok(())
    }
    let mut destinations = Vec::new();
    // The host history directories, and the hook state area the Cursor watch
    // covers: an index there would feed its own writes back as changes.
    let roots = [".claude", ".codex", ".cursor", ".config/memhub-plugin"]
        .map(|name| resolved(&home.join(name)));
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut name = db.as_os_str().to_os_string();
        name.push(suffix);
        let path = PathBuf::from(name);
        let target = resolved(&path).map_err(|_| "Cannot verify index destination")?;
        destinations.push(target.clone());
        for root in &roots {
            if target.starts_with(
                root.as_ref()
                    .map_err(|_| "Cannot verify native source root")?,
            ) {
                return Err("Index database must be outside native history directories");
            }
        }
        #[cfg(unix)]
        if let Ok(metadata) = std::fs::metadata(&path) {
            use std::os::unix::fs::MetadataExt;
            if metadata.nlink() > 1 {
                return Err("Index database or sidecar has multiple hard links");
            }
        }
    }
    for source in [
        ".claude/projects",
        ".codex/sessions",
        ".cursor/chats",
        ".cursor/projects",
        ".config/memhub-plugin/cursorflush",
    ] {
        reject_reverse_aliases(&home.join(source), &destinations)?;
    }
    Ok(())
}
