//! Host registries, shared binary resolution, and Git/GitHub probes.
//!
//! [`environment`] is implemented: it reads a closed, documented list of host
//! configuration files under explicitly supplied roots and reports the
//! components they state. Binary resolution and the Git/GitHub probes remain
//! foundation scaffold; no product API exists for them yet.

pub mod environment;

pub use environment::{
    CacheObservation, CacheSource, ComponentKind, ConfigSource, ConfiguredComponent,
    EnvironmentProbe, IncompleteReason, ProbeHost, ProbeRoots, RootObservation, RootRejected,
    RootScope, RootState, SourceObservation, SourceStatus, UnsupportedReason, probe,
};
