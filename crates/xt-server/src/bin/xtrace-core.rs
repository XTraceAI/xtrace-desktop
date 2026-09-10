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
async fn run() -> Result<(), &'static str> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("serve")) {
        return Err("Usage: xtrace-core serve --db PATH [--port PORT] [--bind 127.0.0.1|::1]");
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
