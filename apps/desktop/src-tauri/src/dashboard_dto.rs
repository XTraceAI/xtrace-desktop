//! Generated IPC shapes stay in the app layer; core metrics have no TypeScript dependency.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricClock {
    System,
    Fixture,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricFavoriteUnknown {
    NoMeasuredOutput,
    UnknownModelOutput,
    UnknownTurnAttribution,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricUsageGap {
    NoSelectedUsage,
    IncompleteCounters,
    UnknownModel,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricInventory {
    FreshComplete,
    Unknown,
    Incomplete,
    Stale,
    MissingPython,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricCaptureGap {
    InventoryUnknown,
    InventoryIncomplete,
    InventoryStale,
    MissingPython,
    StoreUnavailable,
    MissingNativeStart,
    MissingConversationIdentity,
    UnknownSurface,
    DiscoveryIncomplete,
    IdentityMismatch,
    SurfaceMismatch,
    UnresolvedSurfaceDenominator,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricUnpricedReason {
    MissingModel,
    UnknownModel,
    MissingServiceTier,
    UnknownServiceTier,
    MissingCounters,
    MissingPromptCounters,
    MissingCacheSplit,
    InconsistentCacheSplit,
    MissingRate,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardWindow {
    pub days: u32,
    #[ts(type = "number")]
    pub start_ms: i64,
    #[ts(type = "number")]
    pub end_ms: i64,
    pub timezone: String,
    pub clock: MetricClock,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricDelta {
    pub previous: Option<f64>,
    pub pct: Option<f64>,
    pub suppressed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricTile {
    pub value: Option<f64>,
    pub unit: String,
    pub rule_id: String,
    pub reason: Option<String>,
    pub note: Option<String>,
    #[ts(type = "number | null")]
    pub current_n: Option<u64>,
    #[ts(type = "number | null")]
    pub previous_n: Option<u64>,
    pub sample_unit: String,
    pub delta: MetricDelta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardTiles {
    pub agent_hours: MetricTile,
    pub agent_hours_per_day: MetricTile,
    pub human_hours_est: MetricTile,
    pub ratio: MetricTile,
    pub concurrency_max: MetricTile,
    pub concurrency_mean: MetricTile,
    pub hands_off_median: MetricTile,
    pub hands_off_p90: MetricTile,
    pub sessions: MetricTile,
    pub human_messages: MetricTile,
    pub assistant_turns: MetricTile,
    pub tool_calls: MetricTile,
    pub sessions_per_day: MetricTile,
    pub tokens: MetricTile,
    pub cost: MetricTile,
    pub merged_prs: MetricTile,
    pub rule_fires: MetricTile,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricTokenCounters {
    #[ts(type = "number | null")]
    pub input_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub output_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub cache_read_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub cache_creation_tokens: Option<u64>,
    #[ts(type = "number | null")]
    pub total_tokens: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricTokenSummary {
    #[ts(type = "number")]
    pub selected_responses: u64,
    #[ts(type = "number")]
    pub measured_responses: u64,
    #[ts(type = "number")]
    pub sessions: u64,
    #[ts(type = "number")]
    pub measured_sessions: u64,
    pub counters: MetricTokenCounters,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct HostTokenSummary {
    pub host: String,
    pub tokens: MetricTokenSummary,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardDay {
    pub date: String,
    #[ts(type = "number")]
    pub start_ms: i64,
    #[ts(type = "number")]
    pub end_ms: i64,
    pub tokens: MetricTokenSummary,
    pub cost_usd: Option<f64>,
    pub priced_subtotal_usd: f64,
    #[ts(type = "number")]
    pub sessions: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardLane {
    pub session_id: String,
    pub host: String,
    #[ts(type = "number")]
    pub start_ms: i64,
    #[ts(type = "number")]
    pub end_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricFavorite {
    pub model: Option<String>,
    #[ts(type = "number | null")]
    pub output_tokens: Option<u64>,
    pub unknown_reason: Option<MetricFavoriteUnknown>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardFavorite {
    pub current: MetricFavorite,
    pub previous: MetricFavorite,
    pub rule_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricUsageSummary {
    #[ts(type = "number")]
    pub sessions: u64,
    #[ts(type = "number")]
    pub measured: u64,
    pub pct: Option<f64>,
    pub gaps: Vec<MetricUsageGap>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricHostUsage {
    pub host: String,
    pub usage: MetricUsageSummary,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricSurfaceUsage {
    pub host: String,
    pub surface: Option<String>,
    pub usage: MetricUsageSummary,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardUsage {
    pub total: MetricUsageSummary,
    pub by_host: Vec<MetricHostUsage>,
    pub by_surface: Vec<MetricSurfaceUsage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricSurface {
    pub host: String,
    pub surface: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardUsageGate {
    #[ts(type = "number")]
    pub window_start_ms: i64,
    #[ts(type = "number")]
    pub window_end_ms: i64,
    #[ts(type = "number")]
    pub eligible_sessions: u64,
    #[ts(type = "number")]
    pub measured_sessions: u64,
    pub pct: Option<f64>,
    pub passes: Option<bool>,
    pub excluded_surfaces: Vec<MetricSurface>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardCapture {
    pub host: String,
    pub surface: Option<String>,
    pub inventory: MetricInventory,
    #[ts(type = "number")]
    pub observed_sessions: u64,
    #[ts(type = "number")]
    pub captured_sessions: u64,
    #[ts(type = "number")]
    pub unknown_start_sessions: u64,
    pub pct: Option<f64>,
    pub incomplete_reasons: Vec<MetricCaptureGap>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardUnpriced {
    pub model: Option<String>,
    pub service_tier: Option<String>,
    pub reason: MetricUnpricedReason,
    #[ts(type = "number")]
    pub observations: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardCost {
    pub price_version: String,
    pub as_of: String,
    pub basis: String,
    pub total_usd: Option<f64>,
    pub priced_subtotal_usd: f64,
    #[ts(type = "number")]
    pub selected_observations: u64,
    #[ts(type = "number")]
    pub priced_observations: u64,
    #[ts(type = "number")]
    pub unpriced_observations: u64,
    pub unpriced: Vec<DashboardUnpriced>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardUnavailable {
    pub key: String,
    pub reason: String,
}

/// A raw surface excluded from M-09 hands-off by timestamp health.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricExcludedSurface {
    pub host: String,
    pub surface: Option<String>,
    #[ts(type = "number")]
    pub qualifying_sessions: u64,
    #[ts(type = "number")]
    pub degenerate_sessions: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardMetrics {
    pub window: DashboardWindow,
    pub tiles: DashboardTiles,
    pub hands_off_excluded_surfaces: Vec<MetricExcludedSurface>,
    pub favorite: DashboardFavorite,
    pub tokens: MetricTokenSummary,
    pub tokens_by_host: Vec<HostTokenSummary>,
    pub days: Vec<DashboardDay>,
    pub lanes: Vec<DashboardLane>,
    #[ts(type = "number")]
    pub lane_start_ms: i64,
    #[ts(type = "number")]
    pub lane_end_ms: i64,
    #[ts(type = "number")]
    pub lanes_total: u64,
    pub lanes_truncated: bool,
    pub usage_coverage: DashboardUsage,
    pub usage_gate_14d: DashboardUsageGate,
    pub capture_inventory: MetricInventory,
    pub capture_coverage: Vec<DashboardCapture>,
    pub cost: DashboardCost,
    pub unavailable: Vec<DashboardUnavailable>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TokensByHost {
    pub window: DashboardWindow,
    pub hosts: Vec<HostTokenSummary>,
}
