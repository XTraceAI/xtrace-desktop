//! Host registries, shared binary resolution, and Git/GitHub probes.
//!
//! [`environment`] is implemented: it reads a closed, documented list of host
//! configuration files under explicitly supplied roots and reports the
//! components they state.
//!
//! [`gh`] is implemented: it runs one bounded `gh pr view` for one already
//! validated pull-request identity, with the executable path supplied by the
//! caller, and reports the storage layer's typed refresh outcome. Binary
//! resolution and the Git probes remain foundation scaffold; no product API
//! exists for them yet.

pub mod environment;
pub mod gh;

pub use environment::{
    CacheObservation, CacheSource, ComponentKind, ConfigSource, ConfiguredComponent,
    EnvironmentProbe, IncompleteReason, ProbeHost, ProbeRoots, RootObservation, RootRejected,
    RootScope, RootState, SourceObservation, SourceStatus, UnsupportedReason, probe,
};
pub use gh::{AttemptClock, CancelFlag, GhClient, GhClientError, Limits, SystemClock};
