//! Generated IPC shapes for the cached per-PR linked-session report (M-11,
//! M-11a, M-12, M-12a). Each mirrors one `xt_metrics::pr_analytics` type field
//! for field and is reached only through `convert`, so a renamed or removed core
//! field fails instead of silently becoming an unknown value. Nothing here is
//! derived: every row value, count and median comes from the core report.
//!
//! Metric-prefixed so they never collide with the cached inventory's `PrRow` and
//! `PrList`, which describe stored pull requests, not linked-session effort.
use crate::dashboard_dto::{
    DashboardUsageGate, DashboardWindow, MetricExcludedSurface, MetricTokenCounters,
};
use crate::pr_effort_dto::{MetricPrConfidence, MetricPrFreshness, MetricPrFreshnessSummary};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A row's distinct linked sessions by their own link confidence. The row's
/// strongest confidence is a summary, not a claim about each session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricPrLinkEvidence {
    #[ts(type = "number")]
    pub exact: u64,
    #[ts(type = "number")]
    pub sha: u64,
    #[ts(type = "number")]
    pub inferred: u64,
}

/// Sums of the members' M-04 counters; each counter, and the four-counter
/// total, is `null` when any member's is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricPrTokens {
    pub counters: MetricTokenCounters,
    /// Members whose own four-counter total is measured.
    #[ts(type = "number")]
    pub measured_sessions: u64,
    /// Members with no selected usage in the window, active or not.
    #[ts(type = "number")]
    pub no_selected_usage_sessions: u64,
    /// Members with a selected usage observation missing a counter.
    #[ts(type = "number")]
    pub incomplete_sessions: u64,
}

/// M-12a for one row: the median of its members' pooled stretches.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricPrHandsOff {
    /// Pooled stretch count; `null` when a member's stretches are unknown.
    #[ts(type = "number | null")]
    pub n: Option<u64>,
    /// Absent when `n` is zero or unknown; never a zero-minute median.
    pub median_min: Option<f64>,
    #[ts(type = "number")]
    pub measured_sessions: u64,
    #[ts(type = "number")]
    pub unknown_sessions: u64,
    /// Members on a raw surface excluded by window-wide timestamp health.
    #[ts(type = "number")]
    pub excluded_sessions: u64,
    pub excluded_surfaces: Vec<MetricExcludedSurface>,
}

/// One merged pull request's overlapping linked-session effort. Rows are never
/// added together: a session linked to two pull requests counts in both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricPrAnalyticsRow {
    /// Canonical lowercase `owner/repo`; with `number`, the row's identity.
    pub repository: String,
    #[ts(type = "number")]
    pub number: u64,
    pub url: String,
    pub title: Option<String>,
    pub merged_at: String,
    #[ts(type = "number")]
    pub merged_at_ms: i64,
    /// Title token first, then branch; `null` when a required fact is missing.
    pub work_type: Option<String>,
    /// Strongest retained link after the confidence filter.
    pub confidence: MetricPrConfidence,
    pub freshness: MetricPrFreshness,
    /// Distinct indexed user sessions with a retained link, active or not.
    #[ts(type = "number")]
    pub linked_sessions: u64,
    /// The part of `linked_sessions` with a work event in the window.
    #[ts(type = "number")]
    pub active_sessions: u64,
    /// Retained links to an identity that is not an indexed user session.
    #[ts(type = "number")]
    pub unmeasured_links: u64,
    pub evidence: MetricPrLinkEvidence,
    /// M-12; `null` when a member's classification is unknown, or with no members.
    #[ts(type = "number | null")]
    pub human_messages: Option<u64>,
    #[ts(type = "number")]
    pub human_unknown_sessions: u64,
    /// M-11a; parallel sessions add. `null` only with no members.
    #[ts(type = "number | null")]
    pub agent_ms: Option<u64>,
    /// M-11.
    pub tokens: MetricPrTokens,
    pub hands_off: MetricPrHandsOff,
}

/// A per-PR median over one group of rows; n is pull requests.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricPrMedian {
    #[ts(type = "number")]
    pub eligible_prs: u64,
    #[ts(type = "number")]
    pub measured_prs: u64,
    #[ts(type = "number")]
    pub unknown_prs: u64,
    /// Rows with a known absence of any sample (hands-off without stretches).
    #[ts(type = "number")]
    pub no_sample_prs: u64,
    /// Median over the measured PRs only; label it as such.
    pub measured_median: Option<f64>,
    /// The same median, published only when no eligible row is unknown.
    pub median: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricPrTokenWithheld {
    GateFailed,
    GateUnknown,
}

/// Counts always; both medians only when the fixed 14-day gate passes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricPrTokenMedian {
    pub median: MetricPrMedian,
    pub withheld: Option<MetricPrTokenWithheld>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricPrSummary {
    #[ts(type = "number")]
    pub prs: u64,
    pub human_messages: MetricPrMedian,
    pub agent_ms: MetricPrMedian,
    pub tokens: MetricPrTokenMedian,
    /// Median of the rows' pooled-stretch medians, in minutes; n is PRs.
    pub hands_off_min: MetricPrMedian,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricPrTypeSummary {
    /// `null` groups rows whose type is unresolved; it is not `other`.
    pub work_type: Option<String>,
    pub summary: MetricPrSummary,
}

/// What the retained pull requests' cached facts say about the window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricPrEligibility {
    #[ts(type = "number")]
    pub merged: u64,
    #[ts(type = "number")]
    pub outside: u64,
    /// Never refreshed successfully, or `MERGED` without a merge instant.
    #[ts(type = "number")]
    pub unknown_facts: u64,
    #[ts(type = "number")]
    pub unresolved_type: u64,
    pub freshness: MetricPrFreshnessSummary,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricPrAnalyticsReport {
    pub confirmed_only: bool,
    /// The one captured instant the window ends at and the gate is anchored to.
    #[ts(type = "number")]
    pub now_ms: i64,
    pub eligibility: MetricPrEligibility,
    /// Overlapping rows in merge order; never add them together.
    pub rows: Vec<MetricPrAnalyticsRow>,
    pub summary: MetricPrSummary,
    pub by_type: Vec<MetricPrTypeSummary>,
    /// Fixed trailing 14 days ending at `now_ms`, independent of the window.
    pub token_gate: DashboardUsageGate,
    /// Every raw surface M-09 excluded in this window, PR-linked or not.
    pub hands_off_excluded_surfaces: Vec<MetricExcludedSurface>,
}

/// The PRs page's report for one preset and confidence mode, with the event
/// window it measured. A linked-session drilldown pins this `window.end_ms`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct PrAnalyticsPage {
    pub window: DashboardWindow,
    pub report: MetricPrAnalyticsReport,
}

/// One pull request's exact linked-session page as the browser fixture serves
/// it: the request it answers, and the page the native command produced.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct FixturePrSessions {
    pub repository: String,
    #[ts(type = "number")]
    pub number: u64,
    pub confirmed_only: bool,
    pub window_days: u32,
    #[ts(type = "number")]
    pub window_end_ms: i64,
    pub page: crate::dto::SessionPage,
}
