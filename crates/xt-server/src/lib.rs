//! Guarded local transport scaffold. No cloud identity or import acceptance.
mod guard;
mod transport;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    middleware,
    routing::{delete, get, post},
};
use serde_json::json;
use std::{
    future::Future,
    io,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::net::TcpListener;
use xt_store::Store;

pub const DEFAULT_PORT: u16 = 47421;
#[derive(Clone)]
struct AppState {
    port: u16,
    db_path: Arc<PathBuf>,
    // One owner, reserved for the subsequent canonical import adapter.
    _store: Arc<Mutex<Store>>,
}

/// Binding validates the requested literal address before opening a listener.
/// Only exact IPv4/IPv6 localhost is accepted; no DNS resolution is performed.
pub async fn bind(address: SocketAddr) -> io::Result<TcpListener> {
    if address.ip() != std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)
        && address.ip() != std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "server must bind a literal localhost address",
        ));
    }
    TcpListener::bind(address).await
}

/// Serve a prebound, revalidated loopback listener. The guard uses its actual
/// port, including port zero's selected port. Shutdown never owns other processes.
pub async fn serve(
    listener: TcpListener,
    store: Store,
    db_path: PathBuf,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> io::Result<()> {
    let address = listener.local_addr()?;
    if !matches!(address.ip(), std::net::IpAddr::V4(ip) if ip == std::net::Ipv4Addr::LOCALHOST)
        && !matches!(address.ip(), std::net::IpAddr::V6(ip) if ip == std::net::Ipv6Addr::LOCALHOST)
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "listener must be bound to localhost",
        ));
    }
    let state = AppState {
        port: address.port(),
        db_path: Arc::new(db_path),
        _store: Arc::new(Mutex::new(store)),
    };
    let app = Router::new()
        .route("/health", get(health))
        .route("/mcp-server/mcp", post(transport::mcp))
        .route(
            "/v1/developer/access-tokens",
            post(transport::mint).get(transport::tokens),
        )
        .route(
            "/v1/developer/access-tokens/{id}",
            delete(transport::revoke),
        )
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            guard::loopback,
        ))
        .with_state(state);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({"version":env!("CARGO_PKG_VERSION"), "db_path":state.db_path.to_string_lossy()}))
}
