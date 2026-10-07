//! Generated IPC shapes for the Dashboard's M-19 merged-PR tile and effort. Each mirrors one `xt_metrics` report field for field; the dashboard
//! assembler converts through `convert`, so a renamed or removed core field
//! fails instead of silently becoming an unknown value. Nothing here is
//! derived: every count, subtotal and day value comes from the core report.
use crate::pr_dto::PrRefreshErrorCode;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The strongest retained link after the confidence filter (M-13).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricPrConfidence {
    Exact,
    Sha,
    Inferred,
}

/// Cached refresh facts of one pull request. No age policy is applied: a
/// failure after an earlier success keeps that success's facts and says so.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", content = "error", rename_all = "snake_case")]
pub enum MetricPrFreshness {
    NeverAttempted,
    Refreshed,
    FailedNeverRefreshed(PrRefreshErrorCode),
    FailedAfterRefresh(PrRefreshErrorCode),
}

/// Refresh status over every retained pull request the tile considered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricPrFreshnessSummary {
    #[ts(type = "number")]
    pub never_attempted: u64,
    #[ts(type = "number")]
    pub refreshed: u64,
    #[ts(type = "number")]
    pub failed_never_refreshed: u64,
    #[ts(type = "number")]
    pub failed_after_refresh: u64,
    /// Of `failed_never_refreshed`, those a manual refresh failed for since
    /// their last success. The Dashboard does not ask about these again.
    #[ts(type = "number")]
    pub manual_failed_never_refreshed: u64,
    /// Of `failed_after_refresh`, those a manual refresh failed for since
    /// their last success.
    #[ts(type = "number")]
    pub manual_failed_after_refresh: u64,
    /// Oldest last-successful refresh among those pull requests, UTC ms.
    #[ts(type = "number | null")]
    pub oldest_refreshed_at: Option<i64>,
    /// Newest applied attempt, successful or not, UTC ms.
    #[ts(type = "number | null")]
    pub newest_attempted_at: Option<i64>,
}

/// Distinct retained pull requests merged in the window, from cached facts.
///
/// `merged` is `known_merged` only when `complete`; otherwise it is unknown and
/// `known_merged` is a subtotal, never the answer. `unresolved_type` counts
/// merged pull requests whose work type a missing fact leaves open.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricMergedPrs {
    #[ts(type = "number")]
    pub known_merged: u64,
    #[ts(type = "number")]
    pub unknown_facts: u64,
    pub complete: bool,
    #[ts(type = "number | null")]
    pub merged: Option<u64>,
    #[ts(type = "number")]
    pub unresolved_type: u64,
    pub freshness: MetricPrFreshnessSummary,
}

/// One distinct merged pull request on its local merge day. Markers are
/// deduplicated by pull request and are not an effort allocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricPrMarker {
    pub repository: String,
    #[ts(type = "number")]
    pub number: u64,
    pub url: String,
    pub merged_at: String,
    #[ts(type = "number")]
    pub merged_at_ms: i64,
    pub date: String,
    /// `null` when a fact the classification needs is missing.
    pub work_type: Option<String>,
    pub confidence: MetricPrConfidence,
    pub freshness: MetricPrFreshness,
}

/// Where one session's effort is counted. `type` never holds `other`, and
/// `unresolved` is a disclosure, not a work type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", content = "work_type", rename_all = "snake_case")]
pub enum MetricEffortAssignment {
    Type(String),
    Mixed,
    Other,
    Unresolved,
}

/// The selected responses' API-equivalent cost, priced per response exactly
/// as the Dashboard's cost is. `total_usd` is `null` unless every selected
/// response was priced (and at least one was selected); `priced_subtotal_usd`
/// then covers only the priced ones and `unpriced` names the rest, never as
/// zero. `assumed_tier_observations` counts priced Codex responses that
/// recorded no service tier and were priced at OpenAI's default tier.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricEffortCost {
    #[ts(type = "number")]
    pub selected_observations: u64,
    #[ts(type = "number")]
    pub priced_observations: u64,
    #[ts(type = "number")]
    pub unpriced_observations: u64,
    #[ts(type = "number")]
    pub assumed_tier_observations: u64,
    pub total_usd: Option<f64>,
    pub priced_subtotal_usd: f64,
    pub unpriced: Vec<crate::dashboard_dto::DashboardUnpriced>,
}

/// One model's part of one local day. Cost is the priced cost of the
/// responses that named this model on the day; agent time is the time of the
/// sessions whose most-used model over the window is this one, by the rule
/// the Sessions list also shows (`xt_store::session_model`): most selected
/// responses, then most output tokens, then the first name; a session with no
/// response that names a model is judged by its assistant records that name
/// one and carry no usage. `model` is `null` for responses without a model
/// name and for sessions whose in-window work names no model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricModelDayEffort {
    pub model: Option<String>,
    /// Exact priced subtotal in nano-USD; a day's models add up to its priced
    /// subtotal exactly.
    #[ts(type = "number")]
    pub priced_nano_usd: u64,
    #[ts(type = "number")]
    pub priced_observations: u64,
    #[ts(type = "number")]
    pub unpriced_observations: u64,
    #[ts(type = "number")]
    pub agent_ms: u64,
}

/// One local day: cost on each response's event day, agent time allocated to
/// the day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricEffortDay {
    pub date: String,
    #[ts(type = "number")]
    pub start_ms: i64,
    #[ts(type = "number")]
    pub end_ms: i64,
    pub cost: MetricEffortCost,
    #[ts(type = "number")]
    pub agent_ms: u64,
    /// The day's cost and agent time by model, in model-name order (`null`
    /// first); they add up to `cost` and `agent_ms` exactly.
    pub models: Vec<MetricModelDayEffort>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricEffortTotals {
    #[ts(type = "number")]
    pub sessions: u64,
    pub cost: MetricEffortCost,
    /// M-05 active time; parallel sessions add.
    #[ts(type = "number")]
    pub agent_ms: u64,
    pub by_day: Vec<MetricEffortDay>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricAssignmentEffort {
    pub assignment: MetricEffortAssignment,
    pub effort: MetricEffortTotals,
}

/// One window's M-19 report.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricPrEffort {
    pub confirmed_only: bool,
    pub tile: MetricMergedPrs,
    pub markers: Vec<MetricPrMarker>,
    /// Disjoint: each cohort session appears under exactly one assignment.
    pub by_assignment: Vec<MetricAssignmentEffort>,
    /// The whole in-window cohort, linked or not; assignments sum to it.
    pub cohort: MetricEffortTotals,
}

/// The Dashboard's M-19 section: the selected window's full report and the
/// previous equal window's tile, read in the same snapshot as every other
/// Dashboard number. Inferred links are removed before assignment
/// (`confirmed_only`), so only exact and SHA links count.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardPrEffort {
    pub rule_id: String,
    pub current: MetricPrEffort,
    pub previous: MetricMergedPrs,
}

/// A browser fixture's M-19 section for one range, read by the Dashboard
/// assembler after a synthetic refresh.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct FixtureRefreshedPrEffort {
    pub days: u32,
    pub merged_prs: crate::dashboard_dto::MetricTile,
    pub pr_effort: DashboardPrEffort,
}

/// What a fixture database reports after the application's own batch has
/// refreshed exactly `refreshed` (in ascending stored ID order), from its
/// unrefreshed start. The export holds one state per non-empty subset of the
/// listed pull requests, so the browser preview answers every selection it can
/// reach with a report the Rust assembler produced, and never with a number of
/// its own.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct FixturePrEffortState {
    pub refreshed: Vec<crate::pr_dto::PrRef>,
    pub sections: Vec<FixtureRefreshedPrEffort>,
    /// The PRs page report for each range and confidence mode in this state.
    pub analytics: Vec<crate::pr_analytics_dto::PrAnalyticsPage>,
}
