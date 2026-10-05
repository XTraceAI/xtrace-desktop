//! Shared, synthetic fixture inputs with explicit time anchors and honest coverage.
//! A skeleton can validate and export structurally, but cannot pass acceptance.
//! File-backed databases retain their temporary directory for the owner's lifetime.
//! Production crates must not depend on this development crate by default.

mod db;
mod expected;
mod f1;
mod load;
mod registry;

pub use db::TempDb;
pub use load::{
    Fixture, FixtureExport, FixtureStatus, LoadedSession, Manifest, PrLinkInput, SessionInput,
};
pub use registry::{FixtureId, RULE_IDS, RuleId};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{location}: {message}")]
    Invalid { location: String, message: String },
    #[error("{0}: skeleton acceptance is unimplemented")]
    Unimplemented(FixtureId),
    #[error("Fixture filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("Fixture database operation failed: {0}")]
    Store(#[from] xt_store::Error),
    #[error("Fixture export failed: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

fn invalid(location: impl Into<String>, message: impl Into<String>) -> Error {
    Error::Invalid {
        location: location.into(),
        message: message.into(),
    }
}
