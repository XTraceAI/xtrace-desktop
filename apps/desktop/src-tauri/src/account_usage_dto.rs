//! Provider-reported account limits. No credentials or account identifiers cross IPC.
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AccountUsageState {
    Available,
    Unavailable,
    Failed,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AccountUsageIssue {
    NoLogin,
    CredentialAccessRequired,
    CredentialUnavailable,
    Unauthorized,
    RateLimited,
    Timeout,
    Network,
    InvalidResponse,
    SourceUnavailable,
    /// No Claude read has run and none is scheduled (automatic reads are
    /// off). Never a read outcome.
    NotFetched,
    /// The first automatic Claude read is still running and no earlier
    /// reading is saved. Never a read outcome.
    Reading,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum AccountUsageScope {
    AllModels,
    Model,
    Unspecified,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AccountUsageWindow {
    /// Stable provider bucket key, such as `codex` or `claude:sonnet`.
    pub bucket_key: String,
    /// Provider window key, such as `primary`, `secondary`, or `seven_day`.
    pub window_key: String,
    pub scope: AccountUsageScope,
    /// Provider bucket or model name, if supplied. Never hardcode a model list.
    pub name: String,
    /// A readable window label derived from the provider's window name or duration.
    pub window: String,
    /// Percent of this limit already consumed, in [0, 100].
    pub used_percent: f64,
    pub duration_minutes: Option<u32>,
    /// Unix seconds. The renderer formats this in the user's local time zone.
    #[ts(type = "number | null")]
    pub resets_at: Option<i64>,
    /// Where this window is heading by its reset, from the app's own reading
    /// history. Absent while too little of the window has passed, when
    /// nothing is used yet, when the limit is reached, or when the reset is
    /// unknown or has passed. Never saved with the reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pace: Option<AccountUsagePace>,
    /// Use per local calendar day since the window started, for the
    /// all-models week only. Never saved with the reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub daily: Option<Vec<AccountUsageDay>>,
    /// Percent used over the current window, oldest first: the app's saved
    /// readings of this window and then the current reading, thinned to at
    /// most 200 points. For the all-models week only. Never saved with the
    /// reading.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub series: Option<Vec<AccountUsagePoint>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AccountUsagePoint {
    /// Unix seconds: the reading's own read time.
    #[ts(type = "number")]
    pub at: i64,
    pub used_percent: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AccountUsagePace {
    /// Percent used at the reset if use continues at this rate, at most 100.
    pub projected_percent_at_reset: f64,
    /// Percent that would be used by the reading's time if use were spread
    /// evenly over the window: the share of the window passed, times 100.
    pub expected_percent: f64,
    /// Unix seconds when the limit would be reached before the reset, if it would.
    #[ts(type = "number | null")]
    pub run_out_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AccountUsageDay {
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    /// Percentage points of the limit used that day. `null` when the app has
    /// no reading from that day.
    pub used_points: Option<f64>,
    /// The previous day has no reading, so use between the previous reading
    /// and this day's first reading is not counted: `used_points` is a lower bound.
    pub partial: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct AccountProviderUsage {
    pub state: AccountUsageState,
    pub issue: Option<AccountUsageIssue>,
    /// Unix seconds of the last successful provider read.
    #[ts(type = "number | null")]
    pub checked_at: Option<i64>,
    pub windows: Vec<AccountUsageWindow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct AccountUsage {
    pub claude: AccountProviderUsage,
    pub codex: AccountProviderUsage,
}
