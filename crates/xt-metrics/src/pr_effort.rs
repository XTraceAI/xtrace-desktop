//! M-19 merged-PR totals and effort by work type over one selected window.
//!
//! Three independent reads share one snapshot and never add PR rows together:
//!
//! - the tile counts distinct stored pull requests that keep a session link
//!   and whose cached refresh says `MERGED` at a precise `merged_at` inside the
//!   window;
//! - markers list those same pull requests once each, on their local merge day;
//! - the effort cohort is every canonical session with an in-window work event,
//!   linked or not. Each session is assigned exactly once, from its retained
//!   links to pull requests merged in the same window, and its in-window
//!   API-equivalent cost and agent time are added to that one assignment on
//!   the event day.
//!
//! `confidence == inferred` links are removed before anything else when
//! `confirmed_only` is on. Only cached refresh facts are read: no GitHub call,
//! SHA lookup, discovery or refresh happens here, and no age cutoff is applied
//! to cached facts. A failed refresh after an earlier success keeps that
//! success's facts; its status is reported alongside them. A link to a number
//! GitHub said is not a pull request (in a repository it could see, and never
//! confirmed before) is left out of everything here, as if it were not stored.
use crate::{
    CostSummary, Error, MetricsDb, PriceCatalog, Result, Window,
    cost::{self, Totals},
    report::UsageCollector,
    spans::ActiveSpanReport,
};
use jiff::tz::TimeZone;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use xt_store::{
    pr_link::{
        PrConfidence, PrIdentity, PrRefreshError, PrRefreshStatus, PrState, StoredPullRequest,
    },
    timestamp::{self, InstantKey},
};

/// Every link except those to a number GitHub said is not a pull request
/// (`xt_store::not_found_on_github_sql!`): such a link is not a pull request,
/// so it is not counted, not unknown and not in any freshness count.
pub(crate) const LINK_QUERY: &str = concat!(
    "SELECT l.session_id,l.confidence,p.id,p.repo,p.number,
       p.title,p.state,p.merged_at,p.head_ref_name,p.refreshed_at,p.last_attempted_at,p.refresh_error,
       p.manual_failed_at
     FROM pr_links l JOIN pull_requests p ON p.id=l.pr_id
     WHERE NOT ",
    xt_store::not_found_on_github_sql!()
);

// --- ticket-type classification ---------------------------------------------
// The org's own vocabulary: a leading type token from the title
// (conventional-commit style), normalized only across trivial aliases, with the
// first two branch segments as the fallback. Categories are never collapsed.
const TYPE_ALIASES: &[(&str, &str)] = &[
    ("feat", "feat"),
    ("feature", "feat"),
    ("features", "feat"),
    ("fix", "fix"),
    ("fixes", "fix"),
    ("bugfix", "fix"),
    ("bug", "fix"),
    ("hotfix", "hotfix"),
    ("docs", "docs"),
    ("doc", "docs"),
    ("perf", "perf"),
    ("test", "test"),
    ("tests", "test"),
    ("chore", "chore"),
    ("refactor", "refactor"),
    ("ci", "ci"),
    ("build", "build"),
    ("style", "style"),
    ("revert", "revert"),
    ("improvement", "improvement"),
    ("improvements", "improvement"),
];

/// Strip any leading ticket keys (`ENG-863`, `POR-4441/`, `ABC-12:`): one or
/// more of 2–10 ASCII letters, `-`, ASCII digits, then optional whitespace, one
/// optional `:`/`-`/`/` and whitespace, after leading whitespace.
fn strip_tickets(text: &str) -> &str {
    let start = text.trim_start();
    let mut rest = start;
    let mut matched = false;
    loop {
        let letters = rest.bytes().take_while(u8::is_ascii_alphabetic).count();
        let tail = &rest[letters..];
        let digits = tail
            .strip_prefix('-')
            .map_or(0, |t| t.bytes().take_while(u8::is_ascii_digit).count());
        if !(2..=10).contains(&letters) || digits == 0 {
            break;
        }
        let mut after = tail[1 + digits..].trim_start();
        if let Some(separator) = after.strip_prefix([':', '-', '/']) {
            after = separator;
        }
        rest = after.trim_start();
        matched = true;
    }
    if matched { rest } else { text }
}

fn type_token(text: &str) -> Option<&'static str> {
    let text = text.trim();
    let letters = text.bytes().take_while(u8::is_ascii_alphabetic).count();
    let token = text[..letters].to_ascii_lowercase();
    TYPE_ALIASES
        .iter()
        .find(|(alias, _)| *alias == token)
        .map(|(_, category)| *category)
}

fn title_type(title: &str) -> Option<&'static str> {
    let title = strip_tickets(title.trim());
    let title = match title.get(..3) {
        Some(prefix) if prefix.eq_ignore_ascii_case("td/") => title[3..].trim_start(),
        _ => title,
    };
    type_token(title)
}

fn branch_type(head_ref: &str) -> Option<&'static str> {
    head_ref
        .split('/')
        .take(2)
        .find_map(|segment| type_token(strip_tickets(segment)))
}

/// Title token first, branch fallback second, `other` when both are known and
/// neither names a type. A missing fact that the answer depends on is `None`.
pub(crate) fn classify(title: Option<&str>, head_ref: Option<&str>) -> Option<String> {
    if let Some(category) = title.and_then(title_type) {
        return Some(category.to_owned());
    }
    title?;
    Some(branch_type(head_ref?).unwrap_or(OTHER).to_owned())
}

// --- report shapes ----------------------------------------------------------

/// Cached refresh facts only; callers apply their own clock if they need one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "error", rename_all = "snake_case")]
pub enum PrFreshness {
    NeverAttempted,
    Refreshed,
    FailedNeverRefreshed(PrRefreshError),
    FailedAfterRefresh(PrRefreshError),
}

/// Refresh status over every retained pull request the tile considered.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PrFreshnessSummary {
    pub never_attempted: u64,
    pub refreshed: u64,
    pub failed_never_refreshed: u64,
    pub failed_after_refresh: u64,
    /// Of `failed_never_refreshed`, those a manual refresh failed for since
    /// their last success (`StoredPullRequest::manual_failed_at`).
    pub manual_failed_never_refreshed: u64,
    /// Of `failed_after_refresh`, those a manual refresh failed for since
    /// their last success.
    pub manual_failed_after_refresh: u64,
    /// Oldest last-successful refresh among those pull requests, UTC ms.
    pub oldest_refreshed_at: Option<i64>,
    /// Newest applied attempt, successful or not, UTC ms.
    pub newest_attempted_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MergedPrTile {
    /// Distinct retained pull requests known to be merged in the window.
    pub known_merged: u64,
    /// Distinct retained pull requests whose cached facts cannot say whether
    /// they merged in the window: never refreshed successfully, or a `MERGED`
    /// row without its merge instant.
    pub unknown_facts: u64,
    /// True only when no retained pull request has unknown facts.
    pub complete: bool,
    /// `known_merged` when complete, otherwise unknown.
    pub merged: Option<u64>,
    /// Merged-in-window pull requests whose type cannot be classified.
    pub unresolved_type: u64,
    pub freshness: PrFreshnessSummary,
}

/// One distinct merged pull request on its local merge day. Markers are
/// deduplicated by pull request and are not an effort allocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PrMarker {
    pub repository: String,
    pub number: u64,
    pub url: String,
    pub merged_at: String,
    pub merged_at_ms: i64,
    pub date: String,
    /// `None` when a required classification fact is missing.
    pub work_type: Option<String>,
    /// Strongest retained link after the confidence filter.
    pub confidence: PrConfidence,
    pub freshness: PrFreshness,
}

/// Where one session's effort is counted. `Unresolved` is a disclosure, not a
/// work type: a retained link lacks cached facts that could change the answer.
/// `Type` never holds `other`; that is `Other`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(tag = "kind", content = "work_type", rename_all = "snake_case")]
pub enum EffortAssignment {
    Type(String),
    Mixed,
    Other,
    Unresolved,
}

/// One model's part of one local day. Cost is the priced cost of the
/// responses that named this model on the day; agent time is the time of the
/// sessions whose most-used model over the whole window is this one, by the
/// rule the Sessions list shows ([`xt_store::session_model`]). `model` is
/// `None` for responses without a model name and for sessions whose
/// in-window work names no model.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelDayEffort {
    pub model: Option<String>,
    /// Exact priced subtotal in nano-USD; the day's models add up to the
    /// day's priced subtotal exactly.
    pub priced_nano_usd: u64,
    pub priced_observations: u64,
    pub unpriced_observations: u64,
    pub agent_ms: u64,
}

/// One local day: cost on each response's event day, agent time allocated to
/// the day.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DayEffort {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    /// The selected responses priced exactly as the cost report prices them;
    /// unpriced responses are counted and named, never added as zero.
    pub cost: CostSummary,
    pub agent_ms: u64,
    /// The day's cost and agent time by model, in model-name order (`None`
    /// first). They add up to `cost` (priced nano-USD, priced and unpriced
    /// counts) and `agent_ms` exactly. A model appears when it has a response
    /// or agent time on the day.
    pub models: Vec<ModelDayEffort>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EffortTotals {
    pub sessions: u64,
    pub cost: CostSummary,
    /// M-05 active time; parallel sessions add.
    pub agent_ms: u64,
    pub by_day: Vec<DayEffort>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AssignmentEffort {
    pub assignment: EffortAssignment,
    pub effort: EffortTotals,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrEffortReport {
    pub confirmed_only: bool,
    pub tile: MergedPrTile,
    pub markers: Vec<PrMarker>,
    /// Disjoint: each cohort session appears under exactly one assignment.
    pub by_assignment: Vec<AssignmentEffort>,
    /// The whole cohort, ungrouped; assignments sum to it.
    pub cohort: EffortTotals,
}

// --- accumulation -----------------------------------------------------------

#[derive(Default)]
struct ModelTotals {
    nano_usd: u128,
    priced: u64,
    unpriced: u64,
}

struct Accumulator {
    sessions: BTreeSet<String>,
    cost: Totals,
    daily: Vec<Totals>,
    daily_models: Vec<BTreeMap<Option<String>, ModelTotals>>,
    spans: ActiveSpanReport,
}

/// Per session: its in-window model uses.
type SessionModelUse = BTreeMap<String, xt_store::session_model::ModelUses>;

/// Each session's most-used model over the window, by the one rule the
/// Sessions list also shows ([`xt_store::session_model`]). Sessions whose
/// in-window work names no model are absent.
fn dominant_models(usage: &SessionModelUse) -> BTreeMap<String, String> {
    usage
        .iter()
        .filter_map(|(session, uses)| uses.most_used().map(|most| (session.clone(), most.model)))
        .collect()
}

impl Accumulator {
    fn new(days: usize) -> Self {
        Self {
            sessions: BTreeSet::new(),
            cost: Totals::default(),
            daily: vec![Totals::default(); days],
            daily_models: (0..days).map(|_| BTreeMap::new()).collect(),
            spans: ActiveSpanReport {
                spans: Vec::new(),
                active_ms: 0,
            },
        }
    }
    fn push_cost(
        &mut self,
        day: usize,
        observation: &cost::Observation,
        priced: cost::Outcome,
    ) -> Result<()> {
        if matches!(priced, cost::Outcome::Excluded) {
            return Ok(());
        }
        self.cost.push(observation, priced)?;
        self.daily[day].push(observation, priced)?;
        let model = self.daily_models[day]
            .entry(observation.model().map(str::to_owned))
            .or_default();
        match priced {
            cost::Outcome::Excluded => unreachable!("excluded observations return before counting"),
            cost::Outcome::Priced(nano_usd, _) => {
                model.priced += 1;
                model.nano_usd = model
                    .nano_usd
                    .checked_add(nano_usd)
                    .ok_or(Error::CounterOverflow)?;
            }
            cost::Outcome::Unpriced(_) => model.unpriced += 1,
        }
        Ok(())
    }

    fn finish(
        self,
        window: Window,
        zone: TimeZone,
        dominant: &BTreeMap<String, String>,
    ) -> Result<EffortTotals> {
        let agent = self.spans.by_day(window, zone.clone())?;
        // Each session's spans go, whole, to its most-used model; the spans are
        // allocated to days exactly as the day totals allocate them.
        let mut by_model = BTreeMap::<Option<String>, ActiveSpanReport>::new();
        for span in &self.spans.spans {
            by_model
                .entry(dominant.get(&span.session_id).cloned())
                .or_insert_with(|| ActiveSpanReport {
                    spans: Vec::new(),
                    active_ms: 0,
                })
                .spans
                .push(span.clone());
        }
        let mut model_days = Vec::with_capacity(by_model.len());
        for (model, spans) in by_model {
            model_days.push((model, spans.by_day(window, zone.clone())?));
        }
        let mut daily_models = self.daily_models;
        for (model, days) in &model_days {
            for (index, day) in days.iter().enumerate() {
                if day.active_ms > 0 {
                    daily_models[index].entry(model.clone()).or_default();
                }
            }
        }
        let by_day = agent
            .into_iter()
            .zip(self.daily)
            .zip(daily_models)
            .enumerate()
            .map(|(index, ((day, cost), models))| {
                let models = models
                    .into_iter()
                    .map(|(model, totals)| {
                        let agent_ms = model_days
                            .iter()
                            .find(|(name, _)| *name == model)
                            .map_or(0, |(_, days)| days[index].active_ms);
                        Ok(ModelDayEffort {
                            model,
                            priced_nano_usd: u64::try_from(totals.nano_usd)
                                .map_err(|_| Error::CounterOverflow)?,
                            priced_observations: totals.priced,
                            unpriced_observations: totals.unpriced,
                            agent_ms,
                        })
                    })
                    .collect::<Result<_>>()?;
                Ok(DayEffort {
                    date: day.date,
                    start_ms: day.start_ms,
                    end_ms: day.end_ms,
                    cost: cost.finish(),
                    agent_ms: day.active_ms,
                    models,
                })
            })
            .collect::<Result<_>>()?;
        Ok(EffortTotals {
            sessions: self.sessions.len() as u64,
            cost: self.cost.finish(),
            agent_ms: self.spans.active_ms,
            by_day,
        })
    }
}

/// What the cached facts of one linked pull request say about this window.
#[derive(Clone)]
pub(crate) enum Eligibility {
    /// Merged in the window; `None` type means a classification fact is missing.
    Merged(Option<String>),
    /// Conclusively not merged in the window.
    Outside,
    /// No successful refresh (or no merge instant), so eligibility is unknown.
    Unknown,
}

pub(crate) struct LinkedPr {
    pub(crate) stored: StoredPullRequest,
    pub(crate) eligibility: Eligibility,
    pub(crate) merged_at: Option<(InstantKey, i64)>,
    /// Strongest retained link after the confidence filter.
    pub(crate) confidence: PrConfidence,
}

impl From<PrRefreshStatus> for PrFreshness {
    fn from(status: PrRefreshStatus) -> Self {
        match status {
            PrRefreshStatus::NeverAttempted => Self::NeverAttempted,
            PrRefreshStatus::Refreshed => Self::Refreshed,
            PrRefreshStatus::FailedNeverRefreshed(error) => Self::FailedNeverRefreshed(error),
            PrRefreshStatus::FailedAfterRefresh(error) => Self::FailedAfterRefresh(error),
        }
    }
}

fn precise(raw: &str, column: usize) -> rusqlite::Result<(InstantKey, i64)> {
    timestamp::parse(raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

impl PrFreshnessSummary {
    /// Count one retained pull request's cached refresh status.
    pub(crate) fn observe(&mut self, stored: &StoredPullRequest) -> PrFreshness {
        let freshness = PrFreshness::from(stored.refresh_status());
        // The one place a failed pull request is counted as one the user
        // already tried by hand: storage keeps the mark only while the last
        // attempt failed, and clears it on the next success.
        let manual = u64::from(stored.manual_failed_at.is_some());
        match freshness {
            PrFreshness::NeverAttempted => self.never_attempted += 1,
            PrFreshness::Refreshed => self.refreshed += 1,
            PrFreshness::FailedNeverRefreshed(_) => {
                self.failed_never_refreshed += 1;
                self.manual_failed_never_refreshed += manual;
            }
            PrFreshness::FailedAfterRefresh(_) => {
                self.failed_after_refresh += 1;
                self.manual_failed_after_refresh += manual;
            }
        }
        if let Some(at) = stored.refreshed_at {
            self.oldest_refreshed_at = Some(self.oldest_refreshed_at.map_or(at, |old| old.min(at)));
        }
        if let Some(at) = stored.last_attempted_at {
            self.newest_attempted_at = Some(self.newest_attempted_at.map_or(at, |new| new.max(at)));
        }
        freshness
    }
}

/// Every retained link after the confidence filter, one per stored
/// `(session, pull request)`, and the cached facts of the pull requests they
/// name, judged against one window.
pub(crate) struct RetainedLinks {
    pub(crate) prs: BTreeMap<i64, LinkedPr>,
    pub(crate) links: Vec<(String, i64, PrConfidence)>,
}

impl MetricsDb {
    /// One pass over the stored links. `confidence == inferred` links are
    /// removed first under `confirmed_only`, so a pull request kept only by
    /// inferred links is not retained at all.
    pub(crate) fn retained_links(
        &self,
        window: Window,
        confirmed_only: bool,
    ) -> Result<RetainedLinks> {
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let mut prs = BTreeMap::<i64, LinkedPr>::new();
        let mut links = Vec::new();
        let mut statement = self.connection.prepare(LINK_QUERY)?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let confidence: PrConfidence = row.get(1)?;
            if confirmed_only && confidence == PrConfidence::Inferred {
                continue;
            }
            let session: String = row.get(0)?;
            let id: i64 = row.get(2)?;
            links.push((session, id, confidence));
            if let Some(pr) = prs.get_mut(&id) {
                pr.confidence = pr.confidence.strongest(confidence);
                continue;
            }
            let number: i64 = row.get(4)?;
            let number = u64::try_from(number)
                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, number))?;
            let stored = StoredPullRequest {
                id,
                identity: PrIdentity::from_parts(&row.get::<_, String>(3)?, number)?,
                title: row.get(5)?,
                state: row.get(6)?,
                merged_at: row.get(7)?,
                additions: None,
                deletions: None,
                head_ref_name: row.get(8)?,
                refreshed_at: row.get(9)?,
                last_attempted_at: row.get(10)?,
                refresh_error: row.get(11)?,
                manual_failed_at: row.get(12)?,
            };
            let merged_at = stored
                .merged_at
                .as_deref()
                .map(|raw| precise(raw, 7))
                .transpose()?;
            let eligibility = match (stored.state, &merged_at) {
                (Some(PrState::Merged), Some((key, _))) if *key >= start && *key < end => {
                    Eligibility::Merged(classify(
                        stored.title.as_deref(),
                        stored.head_ref_name.as_deref(),
                    ))
                }
                (Some(PrState::Merged), Some(_)) => Eligibility::Outside,
                (Some(PrState::Open | PrState::Closed), _) => Eligibility::Outside,
                (Some(PrState::Merged), None) | (None, _) => Eligibility::Unknown,
            };
            prs.insert(
                id,
                LinkedPr {
                    stored,
                    eligibility,
                    merged_at,
                    confidence,
                },
            );
        }
        Ok(RetainedLinks { prs, links })
    }
}

const OTHER: &str = "other";

/// One session's assignment from its eligible linked pull requests. Two known
/// different types are `mixed` whatever unknown links would add; otherwise any
/// unknown link could change the answer and it is unresolved.
fn assign<'a>(links: impl Iterator<Item = &'a Eligibility>) -> EffortAssignment {
    let mut types = BTreeSet::new();
    let mut undetermined = false;
    for eligibility in links {
        match eligibility {
            Eligibility::Merged(Some(work_type)) => {
                types.insert(work_type.as_str());
            }
            Eligibility::Merged(None) | Eligibility::Unknown => undetermined = true,
            Eligibility::Outside => {}
        }
    }
    if types.len() > 1 {
        return EffortAssignment::Mixed;
    }
    if undetermined {
        return EffortAssignment::Unresolved;
    }
    // An unrecognized PR is classified `other`, the same bucket as a session
    // without eligible links; next to another type it still makes `mixed`.
    match types.pop_first() {
        None | Some(OTHER) => EffortAssignment::Other,
        Some(work_type) => EffortAssignment::Type(work_type.to_owned()),
    }
}

impl MetricsDb {
    /// M-19 over `window`: merged-PR tile, merge-day markers and the disjoint
    /// effort cohort, all from one read snapshot. Sessions come from the M-05
    /// in-window event selection, cost from the global M-04 response selection
    /// priced per response exactly as [`MetricsDb::cost`] prices it, and agent
    /// time from the existing M-05 spans allocated to local days. Nothing is
    /// written.
    pub fn pr_effort(
        &self,
        window: Window,
        zone: TimeZone,
        confirmed_only: bool,
        catalog: &PriceCatalog,
    ) -> Result<PrEffortReport> {
        self.read_snapshot(|db| {
            db.read_pr_effort(window, zone, confirmed_only, catalog, None, None)
        })
    }

    /// [`MetricsDb::pr_effort`] inside the caller's snapshot. `measured` is
    /// this window's [`MetricsDb::active_spans`] already read in that same
    /// snapshot; without it the spans are read here. `shared` also receives
    /// each selected in-window response this report's cost read accepts.
    pub(crate) fn read_pr_effort(
        &self,
        window: Window,
        zone: TimeZone,
        confirmed_only: bool,
        catalog: &PriceCatalog,
        measured: Option<&ActiveSpanReport>,
        mut shared: Option<&mut UsageCollector>,
    ) -> Result<PrEffortReport> {
        let days = window.local_days(zone.clone())?;
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let day_ends: Vec<_> = days
            .iter()
            .map(|day| InstantKey::from_millisecond(day.window.end_ms()))
            .collect();

        let RetainedLinks { prs, links } = self.retained_links(window, confirmed_only)?;
        let mut session_links = BTreeMap::<String, BTreeSet<i64>>::new();
        for (session, id, _) in links {
            session_links.entry(session).or_default().insert(id);
        }

        // Tile and markers: distinct pull requests, independent of effort.
        let mut tile = MergedPrTile {
            known_merged: 0,
            unknown_facts: 0,
            complete: true,
            merged: None,
            unresolved_type: 0,
            freshness: PrFreshnessSummary::default(),
        };
        let mut markers = Vec::new();
        for pr in prs.values() {
            let freshness = tile.freshness.observe(&pr.stored);
            match (&pr.eligibility, &pr.merged_at, &pr.stored.merged_at) {
                (Eligibility::Merged(work_type), Some((key, ms)), Some(raw)) => {
                    tile.known_merged += 1;
                    tile.unresolved_type += u64::from(work_type.is_none());
                    let identity = &pr.stored.identity;
                    markers.push((
                        key,
                        PrMarker {
                            repository: identity.repository().to_owned(),
                            number: identity.number(),
                            url: identity.url(),
                            merged_at: raw.clone(),
                            merged_at_ms: *ms,
                            date: days[day_ends.partition_point(|end| end <= key)]
                                .date
                                .to_string(),
                            work_type: work_type.clone(),
                            confidence: pr.confidence,
                            freshness,
                        },
                    ));
                }
                (Eligibility::Unknown, _, _) => tile.unknown_facts += 1,
                _ => {}
            }
        }
        tile.complete = tile.unknown_facts == 0;
        tile.merged = tile.complete.then_some(tile.known_merged);
        markers.sort_by(|(a, x), (b, y)| {
            a.cmp(b)
                .then_with(|| x.repository.cmp(&y.repository))
                .then_with(|| x.number.cmp(&y.number))
        });
        let markers = markers.into_iter().map(|(_, marker)| marker).collect();

        // Cohort: every session with an in-window work event, assigned once.
        let mut assignments = BTreeMap::<String, EffortAssignment>::new();
        let mut assignment_of = |session: &str| {
            assignments
                .entry(session.to_owned())
                .or_insert_with(|| {
                    assign(
                        session_links
                            .get(session)
                            .into_iter()
                            .flatten()
                            .filter_map(|id| prs.get(id))
                            .map(|pr| &pr.eligibility),
                    )
                })
                .clone()
        };
        let mut cohort = Accumulator::new(days.len());
        let mut grouped = BTreeMap::<EffortAssignment, Accumulator>::new();
        let read;
        let spans = match measured {
            Some(spans) => spans,
            None => {
                read = self.active_spans(window)?;
                &read
            }
        };
        for span in &spans.spans {
            let duration =
                u64::try_from(span.end_ms - span.start_ms).map_err(|_| Error::CounterOverflow)?;
            let group = grouped
                .entry(assignment_of(&span.session_id))
                .or_insert_with(|| Accumulator::new(days.len()));
            for accumulator in [&mut cohort, group] {
                accumulator.sessions.insert(span.session_id.clone());
                accumulator.spans.active_ms = accumulator
                    .spans
                    .active_ms
                    .checked_add(duration)
                    .ok_or(Error::CounterOverflow)?;
                accumulator.spans.spans.push(span.clone());
            }
        }

        // Cost: the global response selection, then the exact window, then
        // each response priced on its own, then the session's single
        // assignment and the response's event day. A selected in-window
        // response is an in-window event of its session, so its session is
        // already in the cohort.
        // The same responses also say which model each session used most.
        let mut usage = SessionModelUse::new();
        let mut statement = self.connection.prepare(cost::SESSION_QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let (ts, surface, observation) = cost::read_row(row)?;
            if ts < start || ts >= end {
                continue;
            }
            let session: String = row.get(11)?;
            let priced = observation.price(catalog, true)?;
            let day = day_ends.partition_point(|end| end <= &ts);
            if let Some(model) = observation.model() {
                usage.entry(session.clone()).or_default().add_responses(
                    model,
                    1,
                    observation.output_tokens().unwrap_or(0),
                );
            }
            let group = grouped
                .entry(assignment_of(&session))
                .or_insert_with(|| Accumulator::new(days.len()));
            for accumulator in [&mut cohort, group] {
                accumulator.sessions.insert(session.clone());
                accumulator.push_cost(day, &observation, priced)?;
            }
            // The same accepted response, for the caller's other reports.
            if let Some(shared) = shared.as_mut() {
                shared.push(&ts, &session, surface.as_deref(), &observation)?;
            }
        }
        drop(rows);
        drop(statement);
        // In-window assistant records that name a model but carry no usage,
        // which the shared rule reads only when no response names a model.
        let mut statement = self
            .connection
            .prepare(xt_store::session_model::UNMETERED_RECORDS_IN_WINDOW_SQL)?;
        let mut rows = statement.query([window.start_ms(), window.candidate_end_ms()?])?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(2)?;
            let ts = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if ts < start || ts >= end {
                continue;
            }
            usage
                .entry(row.get(0)?)
                .or_default()
                .add_unmetered_records(&row.get::<_, String>(1)?, 1);
        }
        let dominant = dominant_models(&usage);

        Ok(PrEffortReport {
            confirmed_only,
            tile,
            markers,
            by_assignment: grouped
                .into_iter()
                .map(|(assignment, accumulator)| {
                    Ok(AssignmentEffort {
                        assignment,
                        effort: accumulator.finish(window, zone.clone(), &dominant)?,
                    })
                })
                .collect::<Result<_>>()?,
            cohort: cohort.finish(window, zone, &dominant)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::classify;
    #[test]
    fn pr_effort_classifier_matches_reference_aliases_and_fallbacks() {
        for (title, head_ref, expected) in [
            (Some("feat: add"), Some("x"), Some("feat")),
            (Some("Feature(ui): add"), None, Some("feat")),
            (Some("features add"), None, Some("feat")),
            (Some("Bugfix thing"), None, Some("fix")),
            (Some("bug: thing"), None, Some("fix")),
            (Some("fixes #1"), None, Some("fix")),
            (Some("hotfix: now"), None, Some("hotfix")),
            (Some("doc: x"), None, Some("docs")),
            (Some("tests: x"), None, Some("test")),
            (Some("improvements x"), None, Some("improvement")),
            (Some("perf: x"), None, Some("perf")),
            (Some("ci: x"), None, Some("ci")),
            (Some("build: x"), None, Some("build")),
            (Some("style: x"), None, Some("style")),
            (Some("revert: x"), None, Some("revert")),
            (Some("chore: x"), None, Some("chore")),
            (Some("refactor: x"), None, Some("refactor")),
            (Some("ENG-863: fix thing"), None, Some("fix")),
            (Some("ENG-863 ABC-12/ docs"), None, Some("docs")),
            (Some("  eng-1 - feat x"), None, Some("feat")),
            (Some("td/ refactor x"), None, Some("refactor")),
            (Some("ENG-1: TD/chore x"), None, Some("chore")),
            // Eleven letters is not a ticket key; the token is not an alias.
            (Some("abcdefghijk-1 fix"), Some("main"), Some("other")),
            (
                Some("POR-4441/Rewards 上線推廣 UI"),
                Some("feat/4441-rewards-launch-ui"),
                Some("feat"),
            ),
            (
                Some("Update"),
                Some("user/eng-866-fix-the-thing"),
                Some("fix"),
            ),
            (Some("Update"), Some("a/b/fix"), Some("other")),
            (Some("Update"), Some("main"), Some("other")),
            (Some("Update"), None, None),
            (None, Some("feat/x"), None),
            (None, None, None),
        ] {
            assert_eq!(
                classify(title, head_ref).as_deref(),
                expected,
                "{title:?} {head_ref:?}"
            );
        }
    }
}
