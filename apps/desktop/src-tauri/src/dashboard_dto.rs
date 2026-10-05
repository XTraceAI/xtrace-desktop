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

/// One reported local day of the selected range. `agent_hours` and
/// `human_hours_est` use the window's M-05 spans and M-07 characters assigned by message timestamp
/// to this day, so the days add up to the hero's totals; `human_hours_est` is
/// `null` on every day of a range whose human time is unknown, and a measured
/// zero stays `0`.
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
    pub agent_hours: f64,
    pub human_hours_est: Option<f64>,
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

/// One session named by the spans this report returns.
///
/// The context is stored session metadata, read by identifier in the same
/// snapshot as the spans and the measurement below it; nothing is derived from
/// a source file here. Every field but the identity and host is independently
/// nullable, because a history can carry a session without any of them, and a
/// session the metadata read found no row for keeps every context field
/// `null` rather than borrowing a zero.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardLaneSession {
    pub session_id: String,
    pub host: String,
    /// The recorded repository, else the working directory it ran in.
    pub repo: Option<String>,
    pub branch: Option<String>,
    /// A nonblank title a source already saved, as Sessions shows it. Never
    /// derived from transcript content.
    pub title: Option<String>,
    /// Verified native Guardian header; a display hint, never a saved title.
    pub automated_review: bool,
    /// The session start in UTC milliseconds, whether or not it falls inside
    /// the lane window: the start the host recorded, or, for a Claude session
    /// (Claude Code records no start), its earliest imported message. `null`
    /// when neither is known; never the first returned span.
    #[ts(type = "number | null")]
    pub started_at_ms: Option<i64>,
    /// Recorded session → pull request links: distinct canonical pull requests
    /// at every evidence level (exact, sha, inferred), over all indexed time,
    /// whatever their merge or refresh state. A measured `0` for an indexed
    /// session with no link; `null` only when no context row was found.
    #[ts(type = "number | null")]
    pub pr_links: Option<u64>,
    /// How many of `pr_links` rest on inferred evidence only; `null` exactly
    /// when `pr_links` is.
    #[ts(type = "number | null")]
    pub inferred_pr_links: Option<u64>,
    /// API-equivalent cost of this session's selected responses over the
    /// whole fixed lane window, not only the spans this report returned: the
    /// display cap never reduces a measurement. The same selection and prices
    /// as the cost tile. `null` when no indexed user session owns the
    /// identifier; never an invented zero.
    pub cost: Option<DashboardLaneCost>,
    /// The session that verifiably created this one, when it is exactly one
    /// indexed user session; `null` otherwise. Display only: it adds no lane,
    /// span or measurement, and a parent outside the returned lanes stays
    /// outside them.
    #[ts(optional = nullable)]
    pub parent: Option<SessionParentLink>,
}

/// One lane session's priced responses over the lane window. A slice of the
/// cost tile's selection: a Claude Code sidechain counts with its parent
/// session, a separately indexed sub-session on its own row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct DashboardLaneCost {
    /// Every selected response was priced; `null` when any was not, or when
    /// none was selected.
    pub total_usd: Option<f64>,
    /// The priced responses only.
    pub priced_subtotal_usd: f64,
    #[ts(type = "number")]
    pub selected_observations: u64,
    #[ts(type = "number")]
    pub priced_observations: u64,
    #[ts(type = "number")]
    pub unpriced_observations: u64,
    /// Priced Codex responses that recorded no service tier, priced at
    /// OpenAI's default tier; included in `priced_observations`.
    #[ts(type = "number")]
    pub assumed_tier_observations: u64,
    /// The responses that could not be priced, by model, service tier and
    /// reason, in the cost report's own shape.
    pub unpriced: Vec<DashboardUnpriced>,
}

/// What kind of structural proof names a session's parent. Closed and
/// evidence-safe: it names the relation's source, never a role, task,
/// prompt or path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SubSessionEvidence {
    /// The host recorded, in this session's own opening metadata, that the
    /// parent session's agent spawned it.
    NativeSpawn,
    /// The parent session's agent launched this session with a recorded
    /// foreground command: that command's own process completed successfully,
    /// the result it wrote named this session, the parent read it, and this
    /// session's first input is what the launch submitted.
    AgentLaunch,
    /// The child's opening metadata identifies a native automated reviewer.
    NativeReviewer,
}

/// The indexed user session verified as the listed session's parent.
///
/// Present only when stored structural evidence resolves by exact
/// host and native identity to exactly one indexed user session other than
/// the child, read in the same snapshot as the row. A parent that is not
/// indexed, not uniquely, or whose relation conflicts is `null`, so a link
/// built from `session_id` always opens the right session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct SessionParentLink {
    /// The parent's exact canonical identity: its own row and detail route.
    pub session_id: String,
    pub host: String,
    /// A nonblank title a source saved for the parent, as its own row shows
    /// it. Never derived from transcript content.
    pub title: Option<String>,
    pub evidence: SubSessionEvidence,
}

impl From<xt_store::session_list::SessionParent> for SessionParentLink {
    fn from(parent: xt_store::session_list::SessionParent) -> Self {
        Self {
            session_id: parent.session_id,
            host: parent.host,
            title: parent.title,
            evidence: match parent.evidence {
                xt_store::session_list::ParentEvidence::NativeSpawn => {
                    SubSessionEvidence::NativeSpawn
                }
                xt_store::session_list::ParentEvidence::AgentLaunch => {
                    SubSessionEvidence::AgentLaunch
                }
                xt_store::session_list::ParentEvidence::NativeReviewer => {
                    SubSessionEvidence::NativeReviewer
                }
            },
        }
    }
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

/// Indexed records that state no timestamp, at one host and one raw source
/// surface. The surface is the value the source stated; `null` stays unknown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct MetricUntimedSurface {
    pub host: String,
    pub surface: Option<String>,
    #[ts(type = "number")]
    pub records: u64,
}

/// All indexed history that no date-based measurement can hold, because those
/// records state no timestamp. It has no window: these records belong to no
/// day, so the count describes every indexed record the app holds and is
/// independent of the selected range and of any filter a table applies.
///
/// `records` is the sum of `by_surface`, which carries one row per observed
/// host and raw surface in host then surface order and omits a pair with no
/// such record. Zero is a measured zero, not a missing fact. Nothing here is a
/// claim that a host is failing, and no timestamp is inferred or repaired.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
pub struct DashboardUntimed {
    #[ts(type = "number")]
    pub records: u64,
    pub by_surface: Vec<MetricUntimedSurface>,
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
    /// Priced Codex responses that recorded no service tier, priced at
    /// OpenAI's default tier; included in `priced_observations`.
    #[ts(type = "number")]
    pub assumed_tier_observations: u64,
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

/// Where a stretch's first tool call sits: the record that carried it and that
/// record's content block index. Mirrors `xt_metrics::ToolBlock` field for
/// field. It is a **position, not an identity** — the native tool_use id is not
/// persisted, and the store's `tool_uses.id` is a row number — so resolving it
/// against a session's verified content is the reader's job, not this one's.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricToolBlock {
    pub record_uuid: String,
    #[ts(type = "number")]
    pub block_index: u64,
}

/// M-20's two thresholds, as a session's stretches were judged against them.
/// Mirrors `xt_metrics::RepeatThresholds`. Both are inclusive.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricRepeatThresholds {
    #[ts(type = "number")]
    pub active_ms: u64,
    #[ts(type = "number")]
    pub repeats: u64,
}

/// Why a stretch's repeated calls could not be counted. Mirrors
/// `xt_metrics::UnknownRepeats`: each names missing evidence, never a measured
/// absence of repeats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MetricUnknownRepeats {
    CallCount,
    MissingBlocks,
    MissingKey,
    UnsupportedVersion,
    ConflictingKey,
}

/// The largest group of identical calls in a stretch: the tool's stored name,
/// how many calls the group holds, and where its earliest call sits. Mirrors
/// `xt_metrics::RepeatGroup`. Nothing about what the calls said crosses — no
/// argument, and not the comparison key that grouped them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricRepeatGroup {
    pub tool_name: String,
    #[ts(type = "number")]
    pub count: u64,
    pub representative: MetricToolBlock,
}

/// What M-20 can say about the calls inside one stretch. Mirrors
/// `xt_metrics::RepeatDensity`. An unknown is never a zero.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MetricRepeatDensity {
    Unknown {
        reason: MetricUnknownRepeats,
    },
    Measured {
        #[ts(type = "number")]
        repeats: u64,
        worst: Option<MetricRepeatGroup>,
    },
}

/// One M-09 stretch of one session, with M-20's answer about the same stretch.
///
/// The first six fields mirror `xt_metrics::SessionStretch`. `duration_ms`
/// is M-09's own duration on the stored millisecond projection. It is carried,
/// never recomputed from `start` and `end`: those are the native spellings as
/// stored, and a duration derived from them could disagree with the one the
/// Dashboard's hands-off number was built from.
///
/// The last three are `xt_metrics::StretchRepeats`' own, read in the same
/// snapshot over the same window. `active_duration_ms` is M-05's fold over the
/// stretch's records — a different measurement from `duration_ms`, not a bound
/// on it — and `circling` is `None` whenever `repeats` is unknown.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct MetricSessionStretch {
    pub start_uuid: String,
    pub end_uuid: String,
    pub start: String,
    pub end: String,
    #[ts(type = "number")]
    pub duration_ms: u64,
    pub first_tool: Option<MetricToolBlock>,
    #[ts(type = "number")]
    pub active_duration_ms: u64,
    pub repeats: MetricRepeatDensity,
    pub circling: Option<bool>,
}

/// What M-09 can say about one session inside one selected window, and M-20
/// about each stretch M-09 states. Mirrors `xt_metrics::SessionStretches`:
/// three different facts, never collapsed into an empty list. M-20's own
/// report has the same three states for the same reasons, and the two are
/// only ever combined when they agree.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum MetricSessionStretches {
    /// No indexed user session owns this identifier.
    Missing,
    /// Indexed, and M-09 states no stretches for it. `excluded_surface` names
    /// the raw surface whose timestamp health excluded it, when that is why.
    Unmeasured {
        excluded_surface: Option<MetricExcludedSurface>,
    },
    /// Every stretch in the window, in chronological order. Empty is honest.
    /// `repeat_thresholds` are the M-20 thresholds each `circling` was judged
    /// against.
    Measured {
        stretches: Vec<MetricSessionStretch>,
        repeat_thresholds: MetricRepeatThresholds,
    },
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
    /// Context and measured output for each distinct session `lanes` names,
    /// ordered by identifier. Not a display order: rows read it by identity.
    pub lane_sessions: Vec<DashboardLaneSession>,
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
    /// All indexed history with no timestamp, independent of `window`.
    pub untimed_history: DashboardUntimed,
    /// M-19 merged pull requests and effort by work type, from cached
    /// refresh facts only; `tiles.merged_prs` states the same tile.
    pub pr_effort: crate::pr_effort_dto::DashboardPrEffort,
    pub cost: DashboardCost,
    pub unavailable: Vec<DashboardUnavailable>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
pub struct TokensByHost {
    pub window: DashboardWindow,
    pub hosts: Vec<HostTokenSummary>,
}

/// One activity-lane span's detail, read on demand when its bar is pointed at
/// or focused. Every field but the prompt's words is `xt_metrics::SpanDetail`
/// field for field; the words may also be read back from the session's
/// original source, in memory, when the index did not keep them, and the
/// source is read at most once for both the prompt and the automatic line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DashboardSpanDetail {
    /// No indexed user session owns this identifier.
    Missing,
    Indexed {
        tool: DashboardSpanTool,
        /// M-04 output tokens of the responses selected inside the span, a
        /// response restated over several records counted once. Unknown when
        /// no selected response stated the counter; a measured zero stays zero.
        #[ts(type = "number | null")]
        output_tokens: Option<u64>,
        prompt: DashboardSpanPrompt,
        /// The latest task notification Claude Code wrote inside the span,
        /// which is automatic and never the person's last message.
        automatic: DashboardSpanAutomatic,
    },
}

/// The tool the span called most often, by M-17's record-bound calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DashboardSpanTool {
    /// The stored tool name and its calls inside the span; a tie goes to the
    /// first name in order.
    Called {
        name: String,
        #[ts(type = "number")]
        calls: u64,
    },
    /// Every record in the span states its tool calls, and there were none.
    NoCalls,
    /// No call is stored, but a record states calls or does not say.
    Unknown,
}

/// The last message a person typed inside the span, else the latest before
/// it in the same session, by the stored M-02 classification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DashboardSpanPrompt {
    /// The session has no person's message at or before the span's end.
    NoMessage,
    /// The newest candidate's classification is unknown.
    Unclassified,
    Found {
        /// When it was recorded, UTC milliseconds.
        #[ts(type = "number")]
        at_ms: i64,
        /// Inside the span, rather than before it started.
        in_span: bool,
        text: DashboardPromptText,
    },
}

/// A chosen message's words. They are the index's own when content retention
/// kept them; otherwise they are read from the session's original source by
/// the record's saved identity, held in memory and never written anywhere.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DashboardPromptText {
    /// One line, whitespace collapsed, bounded in length; `truncated` says
    /// whether more followed.
    Stored { text: String, truncated: bool },
    /// The session's source was read, but no record in it carries this
    /// message's saved identity. No other message is shown in its place.
    NotFound,
    /// More than one record in the session's source carries this message's
    /// saved identity (a fork copies records), and they do not say the same
    /// thing, so neither is shown.
    Ambiguous,
    /// Only part of the message is the person's (a Codex question reply or
    /// image wrapper), and nothing locates that part, so no words are shown.
    Wrapped,
    /// Too many session files are being read for span details right now. The
    /// rest of the detail stands; the words can be asked for again.
    Busy,
    /// The session's source could not be read, for the same reasons a
    /// transcript open names.
    Unavailable {
        reason: crate::dto::SessionSourceReason,
    },
}

/// The latest input inside the span that Claude Code itself wrote as a task
/// notification — a background agent, command or monitor it started finished
/// — by the stored source proof. Automatic, never a person's message.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DashboardSpanAutomatic {
    /// No input inside the span is a proven task notification.
    NoNotification,
    Found {
        /// When it was recorded, UTC milliseconds.
        #[ts(type = "number")]
        at_ms: i64,
        text: DashboardAutomaticText,
    },
}

/// What a task notification announced: its own `<summary>`, from the index
/// when content retention kept it, otherwise read from the session's original
/// source by the record's saved identity, in memory and never written. Every
/// state but `stored` gives no words; the view shows no line for them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum DashboardAutomaticText {
    /// The summary as one line, whitespace collapsed, bounded in length;
    /// `truncated` says whether more followed.
    Stored { text: String, truncated: bool },
    /// The notification's words hold no nonblank summary.
    NoSummary,
    /// The session's source was read, but no record in it carries this
    /// notification's saved identity.
    NotFound,
    /// Several records in the source carry the identity and do not say the
    /// same thing.
    Ambiguous,
    /// Too many session files are being read for span details right now.
    Busy,
    /// The session's source could not be read, for the same reasons a
    /// transcript open names.
    Unavailable {
        reason: crate::dto::SessionSourceReason,
    },
}

/// One span's detail as a fixture carries it, for every span the fixture's
/// Dashboard lanes return.
#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct FixtureSpanDetail {
    pub session_id: String,
    #[ts(type = "number")]
    pub start_ms: i64,
    #[ts(type = "number")]
    pub end_ms: i64,
    pub detail: DashboardSpanDetail,
}
