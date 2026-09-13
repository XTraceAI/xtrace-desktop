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
const USAGE: &str = "Usage: xtrace-core serve --db PATH [--port PORT] [--bind 127.0.0.1|::1]\n       xtrace-core import-native --db PATH --home DIR [--host claude|codex|cursor]... [--pin FILE] [--plugin-root DIR] [--python EXE]";

async fn run() -> Result<(), &'static str> {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref().and_then(std::ffi::OsStr::to_str) {
        Some("serve") => {}
        Some("import-native") => return import_native(args),
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

/// One-shot import of native session history through the pinned shared readers.
/// Prints a JSON report; exits 0 when every requested host imported completely,
/// 2 when a host or session was unavailable, unreadable or incomplete.
fn import_native(mut args: std::iter::Skip<std::env::ArgsOs>) -> Result<(), &'static str> {
    use xt_ingest::native::{ImportRequest, import_native};
    use xt_store::Host;
    let (mut db, mut home, mut pin, mut plugin_root, mut python) = (None, None, None, None, None);
    let mut hosts = Vec::new();
    while let Some(flag) = args.next() {
        let value = args.next().ok_or("An import option is missing its value")?;
        match flag.to_str() {
            Some("--db") if db.is_none() => db = Some(PathBuf::from(value)),
            Some("--home") if home.is_none() => home = Some(PathBuf::from(value)),
            Some("--pin") if pin.is_none() => pin = Some(PathBuf::from(value)),
            Some("--plugin-root") if plugin_root.is_none() => {
                plugin_root = Some(PathBuf::from(value))
            }
            Some("--python") if python.is_none() => python = Some(value),
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
    let observed_at = i64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "Import clock is unavailable")?
            .as_millis(),
    )
    .map_err(|_| "Import clock is unavailable")?;
    let mut store = Store::open(&db).map_err(|_| "Could not open the index database")?;
    let report = import_native(
        &mut store,
        &ImportRequest {
            home: &home,
            hosts: &hosts,
            pin: &pin,
            plugin_root: plugin_root.as_deref(),
            python: python.as_deref(),
            observed_at,
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
