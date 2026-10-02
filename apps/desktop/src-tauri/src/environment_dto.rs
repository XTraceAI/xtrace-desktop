//! Generated Environment IPC shapes. Metric shapes mirror `xt_metrics`'s M-17
//! report and probe shapes mirror `xt_probes::environment`; conversion checks
//! the JSON round trip so a renamed or removed core field fails instead of
//! silently becoming unknown. No shape here carries a filesystem path.
use crate::dashboard_dto::{DashboardWindow, MetricInventory};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One structural tool identity exactly as ingest stored it; `null` details
/// are details the source never stated.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, TS)]
pub struct MetricToolIdentity {
    pub kind: Option<String>,
    pub name: String,
    pub server: Option<String>,
    pub tool: Option<String>,
    pub skill: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricDayCalls {
    pub date: String,
    #[ts(type = "number")]
    pub start_ms: i64,
    #[ts(type = "number")]
    pub end_ms: i64,
    #[ts(type = "number")]
    pub calls: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricIdentityCalls {
    pub identity: MetricToolIdentity,
    #[ts(type = "number")]
    pub calls: u64,
    pub by_day: Vec<MetricDayCalls>,
}

/// Observed calls for one host and one raw source surface; `null` stays unknown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricSurfaceCalls {
    pub host: String,
    pub surface: Option<String>,
    #[ts(type = "number")]
    pub calls: u64,
    pub by_day: Vec<MetricDayCalls>,
    pub by_identity: Vec<MetricIdentityCalls>,
}

/// The M-17 inventory join. The app always supplies an unknown inventory, so
/// `known` never occurs in this bridge; the variant mirrors the core shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricInventoryJoin {
    Known {
        installed: Vec<MetricIdentityCalls>,
        called_not_installed: Vec<MetricIdentityCalls>,
    },
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricHostEnvironment {
    pub host: String,
    pub inventory: MetricInventoryJoin,
    #[ts(type = "number")]
    pub unresolved_calls: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricUnresolvedReason {
    MissingTimestamp,
    UnknownKind,
    UnknownMcpDetail,
    UnknownSkillName,
    HookAttribution,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricUnresolvedCalls {
    pub host: String,
    pub reason: MetricUnresolvedReason,
    #[ts(type = "number")]
    pub calls: u64,
}

/// One M-17 report over one window, exactly as `xt_metrics` computed it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricEnvUsage {
    #[ts(type = "number")]
    pub window_start_ms: i64,
    #[ts(type = "number")]
    pub window_end_ms: i64,
    pub observed: Vec<MetricSurfaceCalls>,
    pub hosts: Vec<MetricHostEnvironment>,
    pub unresolved: Vec<MetricUnresolvedCalls>,
}

/// One identity of one host, aggregated over that host's surfaces, with the
/// selected-range total and the fixed 14-bucket strip. `order` is the
/// deterministic display order Rust assigned; consumers take a top slice or
/// the full list in that order without recounting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EnvIdentityRow {
    pub order: u32,
    pub host: String,
    pub identity: MetricToolIdentity,
    #[ts(type = "number")]
    pub calls: u64,
    #[ts(type = "number")]
    pub strip_calls: u64,
    pub strip: Vec<MetricDayCalls>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EnvTotals {
    #[ts(type = "number")]
    pub selected_calls: u64,
    #[ts(type = "number")]
    pub strip_calls: u64,
    #[ts(type = "number")]
    pub selected_unresolved_calls: u64,
    pub identities: u32,
    pub configured_components: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EnvRootScope {
    Home,
    Repository,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EnvConfigSource {
    ClaudeSkillDirectory,
    ClaudeUserMcpConfig,
    ClaudeProjectMcpConfig,
    ClaudePluginRegistry,
    ClaudeEnabledPlugins,
    ClaudeHookConfig,
    CodexMcpConfig,
    CursorMcpConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EnvCacheSource {
    CodexPluginCache,
    CursorPluginCache,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EnvComponentKind {
    McpServer,
    Skill,
    Plugin,
    Hook,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EnvUnsupportedReason {
    Symlink,
    NotRegularFile,
    NotDirectory,
    FileTooLarge,
    UnsupportedSchema,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EnvIncompleteReason {
    EntryLimit,
    UnprovenEntry,
}

/// `missing` is a supported source that is absent; `empty` is present and
/// states nothing. Neither measures zero installed components.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EnvSourceStatus {
    Missing,
    Empty,
    Read {
        stated: u32,
    },
    Incomplete {
        stated: u32,
        skipped: u32,
        reason: EnvIncompleteReason,
    },
    Malformed,
    Unreadable,
    Unsupported {
        reason: EnvUnsupportedReason,
    },
}

/// A verified configured component: configured, not installed, callable or
/// called. Only structural fields; never a command, argument, value or URL.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EnvConfiguredComponent {
    pub host: String,
    pub source: EnvConfigSource,
    pub scope: EnvRootScope,
    pub kind: EnvComponentKind,
    pub name: String,
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EnvSourceObservation {
    pub host: String,
    pub source: EnvConfigSource,
    pub scope: EnvRootScope,
    pub root_index: u32,
    pub status: EnvSourceStatus,
}

/// A plugin cache directory: a cached download, never an installed component.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EnvCacheObservation {
    pub host: String,
    pub source: EnvCacheSource,
    pub scope: EnvRootScope,
    pub root_index: u32,
    pub status: EnvSourceStatus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum EnvRootState {
    Read,
    Missing,
    NotDirectory,
    Symlink,
    Unreadable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct EnvRootObservation {
    pub scope: EnvRootScope,
    pub index: u32,
    pub state: EnvRootState,
}

/// The Environment bridge. `window` is the selected 7/14/30-day range and
/// `strip_window` the fixed 14 local calendar dates ending in the current
/// partial local day; both use the same clock and zone and are disclosed.
/// `inventory` is always `unknown`: the configured components beside it are
/// what the supported registries verified, not a complete callable inventory,
/// so no not-installed, never-called or used/installed figure is derivable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct EnvironmentMetrics {
    pub window: DashboardWindow,
    pub strip_window: DashboardWindow,
    pub inventory: MetricInventory,
    pub selected: MetricEnvUsage,
    pub strip: MetricEnvUsage,
    pub identities: Vec<EnvIdentityRow>,
    pub totals: EnvTotals,
    pub configured: Vec<EnvConfiguredComponent>,
    pub sources: Vec<EnvSourceObservation>,
    pub cache: Vec<EnvCacheObservation>,
    pub roots: Vec<EnvRootObservation>,
}
