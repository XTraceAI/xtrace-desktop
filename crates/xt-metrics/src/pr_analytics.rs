//! Linked-session effort per merged pull request over one selected window
//! (M-11, M-11a, M-12, M-12a), read-only from cached facts.
//!
//! Every row keeps whole overlapping linked-session effort: a session linked to
//! two pull requests contributes the same in-window measurements to both rows.
//! Rows are never added together, and this report has no total.
//!
//! One read snapshot encloses the retained links and cached pull-request facts,
//! the canonical per-session measurements (read in bounded batches of at most
//! [`MAX_SESSIONS`] identifiers), the window-wide M-09 load and its surface
//! health, and the fixed trailing-14-day token gate. Nothing is written, no
//! GitHub refresh, discovery or source read happens here, and no age cutoff is
//! applied to cached facts.
//!
//! The accepted Stage A contract:
//!
//! - **Eligibility.** A retained pull request is a row when its cached state is
//!   `MERGED` at a precise instant inside the half-open window. Missing facts
//!   are counted as unknown, never as outside or as zero merged.
//! - **Membership.** The distinct indexed user sessions of a row's retained
//!   links after the confidence filter (`linked_sessions`), measured over the
//!   whole event window whether or not they were active in it. Active-in-window
//!   sessions are counted separately; a link to an identity that is not an
//!   indexed user session is disclosed, not measured.
//! - **Unknowns stay unknown.** Human messages and tokens are nullable sums of
//!   [`SessionWindow`] values. A member with no selected usage in the window,
//!   including an inactive one, leaves the token total unknown. An empty window
//!   is still a measured zero for human messages and agent time.
//! - **Hands-off pools stretches.** A row's hands-off median is taken over the
//!   pooled positive M-09 stretches of its members, never over session
//!   medians. Surface health is judged once over every session each raw surface
//!   has in the window. A member on an excluded surface contributes nothing and
//!   is named; a member whose own stretches are unknown makes the row unknown.
//! - **Medians.** Per-PR summaries report the median over measured PRs with the
//!   eligible, measured and unknown PR counts, and publish the unqualified
//!   median only when no eligible PR is unknown. Only token medians wait for
//!   the fixed trailing-14-day token+model gate, anchored to `now_ms`.
use crate::{
    ExcludedSurface, MetricsDb, Result, SessionWindow, TokenCounters, UsageGate, Window, counts,
    hands_off,
    pr_effort::{Eligibility, PrFreshness, PrFreshnessSummary, RetainedLinks},
    session::MAX_SESSIONS,
    stats,
};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use xt_store::pr_link::PrConfidence;

/// Distinct linked sessions of one row by their own retained link confidence.
/// The row's strongest confidence is a summary, not a claim about each session.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct LinkEvidence {
    pub exact: u64,
    pub sha: u64,
    pub inferred: u64,
}

/// Sums of the members' M-04 counters. Each counter, and the four-counter
/// total, is unknown when any member's is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PrTokens {
    pub counters: TokenCounters,
    /// Members whose own four-counter total is measured.
    pub measured_sessions: u64,
    /// Members with no selected usage in the window, active or not.
    pub no_selected_usage_sessions: u64,
    /// Members with a selected usage observation missing a counter.
    pub incomplete_sessions: u64,
}

/// M-12a for one row: the pooled stretches of its measured members.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrHandsOff {
    /// Pooled stretch count; unknown when a member's stretches are.
    pub n: Option<u64>,
    /// Median of the pooled stretches; absent when `n` is zero or unknown.
    pub median_min: Option<f64>,
    /// Members whose stretches are known (possibly none).
    pub measured_sessions: u64,
    /// Members whose own in-window records leave a stretch boundary unknown.
    pub unknown_sessions: u64,
    /// Members on a raw surface excluded by window-wide timestamp health.
    pub excluded_sessions: u64,
    pub excluded_surfaces: Vec<ExcludedSurface>,
}

/// One merged pull request's overlapping linked-session effort.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrRow {
    pub repository: String,
    pub number: u64,
    pub url: String,
    pub title: Option<String>,
    pub merged_at: String,
    pub merged_at_ms: i64,
    /// Title token first, then branch; `None` when a required fact is missing.
    pub work_type: Option<String>,
    /// Strongest retained link after the confidence filter.
    pub confidence: PrConfidence,
    pub freshness: PrFreshness,
    /// Distinct indexed user sessions with a retained link.
    pub linked_sessions: u64,
    /// Linked sessions with at least one work event in the window.
    pub active_sessions: u64,
    /// Retained links whose identity is not an indexed user session.
    pub unmeasured_links: u64,
    pub evidence: LinkEvidence,
    /// M-12; unknown when a member's classification is, or with no members.
    pub human_messages: Option<u64>,
    pub human_unknown_sessions: u64,
    /// M-11a; parallel sessions add. Unknown only with no members.
    pub agent_ms: Option<u64>,
    /// M-11.
    pub tokens: PrTokens,
    pub hands_off: PrHandsOff,
}

/// A per-PR median over one group of rows. n is pull requests.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrMedian {
    pub eligible_prs: u64,
    /// Rows contributing a value.
    pub measured_prs: u64,
    /// Rows whose value is unknown.
    pub unknown_prs: u64,
    /// Rows with a known absence of any sample (hands-off without stretches).
    pub no_sample_prs: u64,
    /// Median over the measured PRs only; label it as such.
    pub measured_median: Option<f64>,
    /// The same median, published only when no eligible row is unknown.
    pub median: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenWithheld {
    GateFailed,
    GateUnknown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TokenMedian {
    /// Counts always; medians only when the fixed 14-day gate passes.
    pub median: PrMedian,
    pub withheld: Option<TokenWithheld>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrSummary {
    pub prs: u64,
    pub human_messages: PrMedian,
    pub agent_ms: PrMedian,
    pub tokens: TokenMedian,
    /// Median of the rows' pooled-stretch medians, in minutes; n is PRs.
    pub hands_off_min: PrMedian,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TypeSummary {
    /// `None` groups rows whose type is unresolved; it is not `other`.
    pub work_type: Option<String>,
    pub summary: PrSummary,
}

/// What the retained pull requests' cached facts say about the window.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PrEligibility {
    /// Rows: merged at a precise instant inside the window.
    pub merged: u64,
    /// Conclusively open, closed or merged outside the window.
    pub outside: u64,
    /// Never refreshed successfully, or `MERGED` without a merge instant.
    pub unknown_facts: u64,
    pub unresolved_type: u64,
    pub freshness: PrFreshnessSummary,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PrAnalyticsReport {
    pub confirmed_only: bool,
    pub now_ms: i64,
    pub eligibility: PrEligibility,
    /// Overlapping rows, in merge order; never add them together.
    pub rows: Vec<PrRow>,
    pub summary: PrSummary,
    pub by_type: Vec<TypeSummary>,
    /// Fixed trailing 14 days ending at `now_ms`, independent of the window.
    pub token_gate: UsageGate,
    /// Every raw surface M-09 excluded in this window, PR-linked or not.
    pub hands_off_excluded_surfaces: Vec<ExcludedSurface>,
}

/// One member's M-09 stretches in the window.
enum Stretches {
    Excluded(ExcludedSurface),
    Unknown,
    Measured(Vec<u64>),
}

enum Value {
    Known(f64),
    NoSample,
    Unknown,
}

fn median(rows: &[&PrRow], value: impl Fn(&PrRow) -> Value) -> PrMedian {
    let mut values = Vec::new();
    let (mut unknown, mut no_sample) = (0_u64, 0_u64);
    for row in rows {
        match value(row) {
            Value::Known(value) => values.push(value),
            Value::NoSample => no_sample += 1,
            Value::Unknown => unknown += 1,
        }
    }
    let measured_median = stats::median_f64(&mut values);
    PrMedian {
        eligible_prs: rows.len() as u64,
        measured_prs: values.len() as u64,
        unknown_prs: unknown,
        no_sample_prs: no_sample,
        measured_median,
        median: measured_median.filter(|_| unknown == 0),
    }
}

fn counted(value: Option<u64>) -> Value {
    value.map_or(Value::Unknown, |value| Value::Known(value as f64))
}

fn summarize(rows: &[&PrRow], gate: &UsageGate) -> PrSummary {
    let mut tokens = median(rows, |row| counted(row.tokens.counters.total_tokens));
    let withheld = match gate.passes {
        Some(true) => None,
        Some(false) => Some(TokenWithheld::GateFailed),
        None => Some(TokenWithheld::GateUnknown),
    };
    if withheld.is_some() {
        tokens.measured_median = None;
        tokens.median = None;
    }
    PrSummary {
        prs: rows.len() as u64,
        human_messages: median(rows, |row| counted(row.human_messages)),
        agent_ms: median(rows, |row| counted(row.agent_ms)),
        tokens: TokenMedian {
            median: tokens,
            withheld,
        },
        // No stretches is a known absence only when no member was excluded:
        // an excluded surface's stretches exist but cannot be measured.
        hands_off_min: median(rows, |row| {
            match (row.hands_off.n, row.hands_off.median_min) {
                (Some(_), Some(median)) => Value::Known(median),
                (Some(_), None) if row.hands_off.excluded_sessions == 0 => Value::NoSample,
                _ => Value::Unknown,
            }
        }),
    }
}

impl MetricsDb {
    /// Stage A per-PR report over `window`, with the token gate anchored to the
    /// caller's single captured `now_ms`. All reads share one snapshot.
    pub fn pr_analytics(
        &self,
        window: Window,
        now_ms: i64,
        confirmed_only: bool,
    ) -> Result<PrAnalyticsReport> {
        self.read_snapshot(|db| db.read_pr_analytics(window, now_ms, confirmed_only))
    }

    fn read_pr_analytics(
        &self,
        window: Window,
        now_ms: i64,
        confirmed_only: bool,
    ) -> Result<PrAnalyticsReport> {
        let RetainedLinks { prs, links } = self.retained_links(window, confirmed_only)?;
        let mut eligibility = PrEligibility {
            merged: 0,
            outside: 0,
            unknown_facts: 0,
            unresolved_type: 0,
            freshness: PrFreshnessSummary::default(),
        };
        let mut freshness = BTreeMap::new();
        for (id, pr) in &prs {
            freshness.insert(*id, eligibility.freshness.observe(&pr.stored));
            match &pr.eligibility {
                Eligibility::Merged(work_type) => {
                    eligibility.merged += 1;
                    eligibility.unresolved_type += u64::from(work_type.is_none());
                }
                Eligibility::Outside => eligibility.outside += 1,
                Eligibility::Unknown => eligibility.unknown_facts += 1,
            }
        }

        // Eligible rows' links; the store keeps one link per (session, PR).
        let mut members = BTreeMap::<i64, BTreeMap<String, PrConfidence>>::new();
        for (session, id, confidence) in links {
            if matches!(
                prs.get(&id).map(|pr| &pr.eligibility),
                Some(Eligibility::Merged(_))
            ) {
                members.entry(id).or_default().insert(session, confidence);
            }
        }
        let distinct: Vec<&str> = members
            .values()
            .flat_map(BTreeMap::keys)
            .map(String::as_str)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        // Canonical per-session measurements, once per distinct session.
        let mut measured = BTreeMap::<String, SessionWindow>::new();
        let mut surfaces = BTreeMap::<String, (String, Option<String>)>::new();
        for batch in distinct.chunks(MAX_SESSIONS) {
            measured.extend(self.session_windows(window, batch)?);
            surfaces.extend(self.session_surfaces(batch)?);
        }

        // M-09: one window-wide load, health judged over every session of each
        // raw surface, then each member's stretches folded once.
        let loaded = hands_off::load(&self.connection, window)?;
        let health: BTreeMap<_, _> = loaded
            .iter()
            .map(|((host, surface), sessions)| {
                (
                    (host.clone(), surface.clone()),
                    hands_off::health(host, surface, sessions),
                )
            })
            .collect();
        let mut stretches = BTreeMap::<&str, Stretches>::new();
        for (id, key) in &surfaces {
            let state = if let Some(Some(excluded)) = health.get(key) {
                Stretches::Excluded(excluded.clone())
            } else {
                match loaded.get(key).and_then(|sessions| sessions.get(id)) {
                    None => Stretches::Measured(Vec::new()),
                    Some(events) => match hands_off::collect(events)? {
                        None => Stretches::Unknown,
                        Some(found) => {
                            Stretches::Measured(found.iter().map(|s| s.duration_ms()).collect())
                        }
                    },
                }
            };
            stretches.insert(id.as_str(), state);
        }

        let mut rows = Vec::new();
        for (id, sessions) in &members {
            let pr = &prs[id];
            let (Eligibility::Merged(work_type), Some((key, merged_at_ms)), Some(merged_at)) =
                (&pr.eligibility, &pr.merged_at, &pr.stored.merged_at)
            else {
                continue;
            };
            let mut row = PrRow {
                repository: pr.stored.identity.repository().to_owned(),
                number: pr.stored.identity.number(),
                url: pr.stored.identity.url(),
                title: pr.stored.title.clone(),
                merged_at: merged_at.clone(),
                merged_at_ms: *merged_at_ms,
                work_type: work_type.clone(),
                confidence: pr.confidence,
                freshness: freshness[id],
                linked_sessions: 0,
                active_sessions: 0,
                unmeasured_links: 0,
                evidence: LinkEvidence::default(),
                human_messages: None,
                human_unknown_sessions: 0,
                agent_ms: None,
                tokens: PrTokens {
                    counters: TokenCounters {
                        input_tokens: None,
                        output_tokens: None,
                        cache_read_tokens: None,
                        cache_creation_tokens: None,
                        total_tokens: None,
                    },
                    measured_sessions: 0,
                    no_selected_usage_sessions: 0,
                    incomplete_sessions: 0,
                },
                hands_off: PrHandsOff {
                    n: None,
                    median_min: None,
                    measured_sessions: 0,
                    unknown_sessions: 0,
                    excluded_sessions: 0,
                    excluded_surfaces: Vec::new(),
                },
            };
            let mut pooled = Vec::new();
            let mut excluded = BTreeSet::new();
            for (session, confidence) in sessions {
                let Some(SessionWindow::Indexed {
                    events,
                    human_messages,
                    tokens,
                    agent_ms,
                    ..
                }) = measured.get(session)
                else {
                    row.unmeasured_links += 1;
                    continue;
                };
                // The first member turns every nullable sum into a measured zero.
                if row.linked_sessions == 0 {
                    row.human_messages = Some(0);
                    row.agent_ms = Some(0);
                    let counters = &mut row.tokens.counters;
                    for counter in [
                        &mut counters.input_tokens,
                        &mut counters.output_tokens,
                        &mut counters.cache_read_tokens,
                        &mut counters.cache_creation_tokens,
                        &mut counters.total_tokens,
                    ] {
                        *counter = Some(0);
                    }
                }
                row.linked_sessions += 1;
                row.active_sessions += u64::from(*events > 0);
                match confidence {
                    PrConfidence::Exact => row.evidence.exact += 1,
                    PrConfidence::Sha => row.evidence.sha += 1,
                    PrConfidence::Inferred => row.evidence.inferred += 1,
                }
                row.human_messages = counts::add(row.human_messages, *human_messages)?;
                row.human_unknown_sessions += u64::from(human_messages.is_none());
                row.agent_ms = counts::add(row.agent_ms, Some(*agent_ms))?;
                let (sum, add) = (&mut row.tokens.counters, &tokens.counters);
                sum.input_tokens = counts::add(sum.input_tokens, add.input_tokens)?;
                sum.output_tokens = counts::add(sum.output_tokens, add.output_tokens)?;
                sum.cache_read_tokens = counts::add(sum.cache_read_tokens, add.cache_read_tokens)?;
                sum.cache_creation_tokens =
                    counts::add(sum.cache_creation_tokens, add.cache_creation_tokens)?;
                sum.total_tokens = counts::add(sum.total_tokens, add.total_tokens)?;
                row.tokens.measured_sessions += u64::from(add.total_tokens.is_some());
                if tokens.selected_responses == 0 {
                    row.tokens.no_selected_usage_sessions += 1;
                } else if tokens.measured_responses < tokens.selected_responses {
                    row.tokens.incomplete_sessions += 1;
                }
                match stretches.get(session.as_str()) {
                    Some(Stretches::Measured(durations)) => {
                        row.hands_off.measured_sessions += 1;
                        pooled.extend_from_slice(durations);
                    }
                    Some(Stretches::Excluded(surface)) => {
                        row.hands_off.excluded_sessions += 1;
                        excluded.insert((surface.host.clone(), surface.surface.clone()));
                    }
                    // The surface read shares this snapshot, so an indexed
                    // session always has one; stay unknown rather than guess.
                    Some(Stretches::Unknown) | None => row.hands_off.unknown_sessions += 1,
                }
            }
            if row.linked_sessions > 0 && row.hands_off.unknown_sessions == 0 {
                row.hands_off.n = Some(pooled.len() as u64);
                row.hands_off.median_min =
                    stats::median_p90(&mut pooled).map(|(median, _)| median / 60000.0);
            }
            row.hands_off.excluded_surfaces = excluded
                .iter()
                .filter_map(|key| health.get(key).cloned().flatten())
                .collect();
            rows.push((key.clone(), row));
        }
        rows.sort_by(|(a, x), (b, y)| {
            a.cmp(b)
                .then_with(|| x.repository.cmp(&y.repository))
                .then_with(|| x.number.cmp(&y.number))
        });
        let rows: Vec<PrRow> = rows.into_iter().map(|(_, row)| row).collect();

        let token_gate = self.usage_gate(now_ms)?;
        let all: Vec<&PrRow> = rows.iter().collect();
        let mut types = BTreeMap::<Option<&str>, Vec<&PrRow>>::new();
        for row in &rows {
            types.entry(row.work_type.as_deref()).or_default().push(row);
        }
        Ok(PrAnalyticsReport {
            confirmed_only,
            now_ms,
            eligibility,
            summary: summarize(&all, &token_gate),
            by_type: types
                .into_iter()
                .map(|(work_type, rows)| TypeSummary {
                    work_type: work_type.map(str::to_owned),
                    summary: summarize(&rows, &token_gate),
                })
                .collect(),
            rows,
            token_gate,
            hands_off_excluded_surfaces: health.into_values().flatten().collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, functions::FunctionFlags};
    use xt_store::{
        CanonicalRecord, SessionMeta, SessionSource,
        pr_link::{PrIdentity, PrLinkObservation, PrState, RefreshOutcome, RefreshSuccess},
    };

    #[test]
    fn links_facts_measurements_and_gate_read_one_snapshot_during_a_commit() {
        let mut db = xt_fixtures::TempDb::empty().unwrap();
        let mut session = SessionMeta::new("s", "claude", SessionSource::Fixture);
        session.surface = Some("cli".into());
        db.store_mut().upsert_session(&session, false).unwrap();
        let rows: Vec<CanonicalRecord> = [
            serde_json::json!({"uuid":"h","type":"user","timestamp":"2026-09-07T12:00:00Z","message":{"role":"user","content":[{"type":"text","text":"Synthetic"}]}}),
            serde_json::json!({"uuid":"t","type":"assistant","timestamp":"2026-09-07T12:01:00Z","message":{"role":"assistant","model":"m","content":[{"type":"tool_use","id":"x","name":"Read","input":{}}],"usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}),
        ]
        .into_iter()
        .map(|value| serde_json::from_value(value).unwrap())
        .collect();
        db.store_mut().upsert_records("s", &rows, false).unwrap();
        let identity = PrIdentity::from_url("https://github.com/xtrace/app/pull/1").unwrap();
        db.store_mut()
            .record_pr_link(&PrLinkObservation {
                session_id: "s".into(),
                pull_request: identity.clone(),
                confidence: PrConfidence::Exact,
                first_seen_at: 1,
                last_seen_at: 2,
            })
            .unwrap();
        db.store_mut()
            .record_pr_refresh(&RefreshOutcome::Success(RefreshSuccess {
                pull_request: identity,
                attempted_at: 100,
                title: "feat: x".into(),
                state: PrState::Merged,
                merged_at: Some("2026-09-05T00:00:00Z".into()),
                additions: 1,
                deletions: 1,
                head_ref_name: "main".into(),
            }))
            .unwrap();

        let metrics = MetricsDb::open(db.path()).unwrap();
        let writer = Connection::open(db.path()).unwrap();
        // The links are read before any event, so a commit landing on the
        // first event read would mix revisions if the report were not one
        // snapshot.
        writer.execute_batch("DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT session_id,host,surface,ts_ms,pr_snapshot_write(ts) AS ts,uuid,type,is_human,text_len,human_is_eligible,human_text_len,human_excluded,role,tool_use_count,confirmed_automated_input FROM v_records WHERE ts_ms IS NOT NULL").unwrap();
        let path = db.path().to_owned();
        let written = std::sync::atomic::AtomicBool::new(false);
        metrics
            .connection
            .create_scalar_function("pr_snapshot_write", 1, FunctionFlags::SQLITE_UTF8, move |context| {
                if !written.swap(true, std::sync::atomic::Ordering::SeqCst) {
                    Connection::open(&path)?.execute_batch(
                        "UPDATE usage SET output_tokens=NULL; UPDATE records SET is_human=NULL; DELETE FROM pr_links;",
                    )?;
                }
                context.get::<String>(0)
            })
            .unwrap();
        let window = Window::new(1788220800000, 1788825600000).unwrap();
        let during = metrics
            .pr_analytics(window, window.end_ms(), false)
            .unwrap();
        assert!(metrics.connection.is_autocommit());
        assert_eq!(during.rows.len(), 1);
        let row = &during.rows[0];
        assert_eq!(row.tokens.counters.total_tokens, Some(3));
        assert_eq!(row.human_messages, Some(1));
        assert_eq!(
            (row.hands_off.n, row.hands_off.median_min),
            (Some(1), Some(1.0))
        );
        assert_eq!(during.token_gate.passes, Some(true));

        let after = metrics
            .pr_analytics(window, window.end_ms(), false)
            .unwrap();
        assert!(after.rows.is_empty());
        assert_eq!(after.token_gate.passes, Some(false));
    }
}
