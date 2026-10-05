use crate::{Error, MetricsDb, Result, Window};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use xt_store::timestamp::{self, InstantKey};

pub(crate) const EVENTS_QUERY: &str =
    "SELECT session_id,host,surface,ts FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";
pub(crate) const USAGE_QUERY: &str = "SELECT session_id,ts,model,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens FROM v_response_usage WHERE ts_ms>=?1 AND ts_ms<?2";
pub(crate) const CAPTURE_QUERY: &str = "SELECT d.host,d.surface,d.started_at_ms,d.conversation_id,d.discovery_complete,s.host,s.surface,
    EXISTS(SELECT 1 FROM capture_receipts r WHERE r.session_id=d.conversation_id AND r.surface=d.surface AND r.coverage_sealed=1)
    FROM discovered_sessions d LEFT JOIN sessions s ON s.session_id=d.conversation_id
    WHERE (d.started_at_ms>=?1 AND d.started_at_ms<?2) OR d.started_at_ms IS NULL";

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct CoverageSurface {
    pub host: String,
    pub surface: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageGap {
    NoSelectedUsage,
    IncompleteCounters,
    UnknownModel,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct UsageCoverageSummary {
    pub sessions: u64,
    pub measured: u64,
    pub pct: Option<f64>,
    pub gaps: Vec<UsageGap>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HostUsageCoverage {
    pub host: String,
    pub usage: UsageCoverageSummary,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SurfaceUsageCoverage {
    pub host: String,
    pub surface: Option<String>,
    pub usage: UsageCoverageSummary,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsageCoverage {
    pub total: UsageCoverageSummary,
    pub by_host: Vec<HostUsageCoverage>,
    pub by_surface: Vec<SurfaceUsageCoverage>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct UsageGate {
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub eligible_sessions: u64,
    pub measured_sessions: u64,
    pub pct: Option<f64>,
    pub passes: Option<bool>,
    pub excluded_surfaces: Vec<CoverageSurface>,
}

/// Supplied by the runtime that owns discovery. FreshComplete is an explicit
/// assertion of a complete, fresh inventory; stored row flags cannot establish it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventoryState {
    FreshComplete,
    #[default]
    Unknown,
    Incomplete,
    Stale,
    MissingPython,
    Unavailable,
}
#[derive(Clone, Debug)]
pub struct DiscoveryHealth {
    pub host: String,
    pub surface: Option<String>,
    pub inventory: InventoryState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureGap {
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
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SurfaceCapture {
    pub host: String,
    pub surface: Option<String>,
    pub inventory: InventoryState,
    /// Distinct discovered native sessions with known starts in the window.
    pub observed_sessions: u64,
    pub captured_sessions: u64,
    pub unknown_start_sessions: u64,
    pub pct: Option<f64>,
    pub incomplete_reasons: Vec<CaptureGap>,
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Coverage {
    pub usage: UsageCoverage,
    pub gate_14d: UsageGate,
    pub capture: Vec<SurfaceCapture>,
}

fn pct(n: u64, d: u64) -> Option<f64> {
    (d > 0).then(|| n as f64 * 100.0 / d as f64)
}
fn in_window(raw: &str, window: Window) -> rusqlite::Result<bool> {
    let instant = timestamp::parse(raw)
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e))
        })?
        .0;
    Ok(instant >= InstantKey::from_millisecond(window.start_ms())
        && instant < InstantKey::from_millisecond(window.end_ms()))
}
fn known(value: Option<&str>) -> bool {
    value.is_some_and(|s| !s.trim().is_empty())
}
struct SessionUsage {
    surface: CoverageSurface,
    observations: usize,
    gaps: BTreeSet<UsageGap>,
}
impl UsageCoverageSummary {
    fn add(&mut self, session: &SessionUsage) {
        self.sessions += 1;
        self.measured += u64::from(session.gaps.is_empty());
        self.gaps.extend(session.gaps.iter().copied());
        self.gaps.sort();
        self.gaps.dedup();
        self.pct = pct(self.measured, self.sessions);
    }
}

impl MetricsDb {
    /// One read snapshot for display coverage, fixed trailing-14-day gate and
    /// native-start capture cohorts. This does not scan or refresh discovery.
    pub fn coverage(
        &self,
        window: Window,
        now_ms: i64,
        health: &[DiscoveryHealth],
    ) -> Result<Coverage> {
        let snapshot = self
            .connection
            .is_autocommit()
            .then(|| self.connection.unchecked_transaction())
            .transpose()?;
        let display = self.usage_sessions(window)?;
        let mut total = UsageCoverageSummary::default();
        let mut hosts = BTreeMap::<String, UsageCoverageSummary>::new();
        let mut surfaces = BTreeMap::<CoverageSurface, UsageCoverageSummary>::new();
        for session in display.values() {
            total.add(session);
            hosts
                .entry(session.surface.host.clone())
                .or_default()
                .add(session);
            surfaces
                .entry(session.surface.clone())
                .or_default()
                .add(session);
        }
        let usage = UsageCoverage {
            total,
            by_host: hosts
                .into_iter()
                .map(|(host, usage)| HostUsageCoverage { host, usage })
                .collect(),
            by_surface: surfaces
                .into_iter()
                .map(|(s, usage)| SurfaceUsageCoverage {
                    host: s.host,
                    surface: s.surface,
                    usage,
                })
                .collect(),
        };
        let gate_14d = self.usage_gate(now_ms)?;
        let capture = self.capture_coverage(window, health)?;
        if let Some(snapshot) = snapshot {
            snapshot.commit()?;
        }
        Ok(Coverage {
            usage,
            gate_14d,
            capture,
        })
    }

    /// M-11's token+model gate over the fixed trailing 14 days ending at the
    /// caller's `now_ms`, never the display window. Structurally unmeasured
    /// surfaces leave the denominator and stay named; an empty denominator is
    /// unknown, and exactly 90% passes.
    pub(crate) fn usage_gate(&self, now_ms: i64) -> Result<UsageGate> {
        let fixed = Window::new(
            now_ms
                .checked_sub(14 * 24 * 60 * 60 * 1000)
                .ok_or(Error::InvalidWindow)?,
            now_ms,
        )?;
        let mut eligible = UsageCoverageSummary::default();
        let mut excluded = BTreeSet::new();
        for session in self.usage_sessions(fixed)?.values() {
            if session.surface.host == "cursor"
                && session.surface.surface.as_deref() == Some("cursor-cli")
            {
                excluded.insert(session.surface.clone());
            } else {
                eligible.add(session);
            }
        }
        Ok(UsageGate {
            window_start_ms: fixed.start_ms(),
            window_end_ms: fixed.end_ms(),
            eligible_sessions: eligible.sessions,
            measured_sessions: eligible.measured,
            pct: eligible.pct,
            passes: (eligible.sessions > 0)
                .then(|| u128::from(eligible.measured) * 10 >= u128::from(eligible.sessions) * 9),
            excluded_surfaces: excluded.into_iter().collect(),
        })
    }

    fn usage_sessions(&self, window: Window) -> Result<BTreeMap<String, SessionUsage>> {
        let mut sessions = BTreeMap::new();
        let bounds = [window.start_ms(), window.candidate_end_ms()?];
        let mut statement = self.connection.prepare(EVENTS_QUERY)?;
        let mut rows = statement.query(bounds)?;
        while let Some(row) = rows.next()? {
            if !in_window(&row.get::<_, String>(3)?, window)? {
                continue;
            }
            sessions
                .entry(row.get::<_, String>(0)?)
                .or_insert(SessionUsage {
                    surface: CoverageSurface {
                        host: row.get(1)?,
                        surface: row.get(2)?,
                    },
                    observations: 0,
                    gaps: BTreeSet::new(),
                });
        }
        let mut statement = self.connection.prepare(USAGE_QUERY)?;
        let mut rows = statement.query(bounds)?;
        while let Some(row) = rows.next()? {
            if !in_window(&row.get::<_, String>(1)?, window)? {
                continue;
            }
            let id: String = row.get(0)?;
            let Some(session) = sessions.get_mut(&id) else {
                continue;
            };
            session.observations += 1;
            let model: Option<String> = row.get(2)?;
            if !known(model.as_deref()) {
                session.gaps.insert(UsageGap::UnknownModel);
            }
            for column in 3..7 {
                if row.get::<_, Option<i64>>(column)?.is_none() {
                    session.gaps.insert(UsageGap::IncompleteCounters);
                }
            }
        }
        for session in sessions.values_mut() {
            if session.observations == 0 {
                session.gaps.insert(UsageGap::NoSelectedUsage);
            }
        }
        Ok(sessions)
    }

    fn capture_coverage(
        &self,
        window: Window,
        health: &[DiscoveryHealth],
    ) -> Result<Vec<SurfaceCapture>> {
        let mut groups = BTreeMap::<CoverageSurface, SurfaceCapture>::new();
        for context in health {
            let key = CoverageSurface {
                host: context.host.clone(),
                surface: context.surface.clone(),
            };
            if groups
                .insert(key.clone(), SurfaceCapture::new(key, context.inventory))
                .is_some()
            {
                return Err(Error::InvalidCoverageContext);
            }
        }
        let mut uncertain_hosts = BTreeSet::new();
        let mut statement = self.connection.prepare(CAPTURE_QUERY)?;
        let mut rows = statement.query([window.start_ms(), window.end_ms()])?;
        while let Some(row) = rows.next()? {
            let key = CoverageSurface {
                host: row.get(0)?,
                surface: row.get(1)?,
            };
            if !known(key.surface.as_deref()) {
                uncertain_hosts.insert(key.host.clone());
            }
            let group = groups
                .entry(key.clone())
                .or_insert_with(|| SurfaceCapture::new(key, InventoryState::Unknown));
            let start: Option<i64> = row.get(2)?;
            let conversation: Option<String> = row.get(3)?;
            if start.is_some() {
                group.observed_sessions += 1;
            } else {
                group.unknown_start_sessions += 1;
                group
                    .incomplete_reasons
                    .push(CaptureGap::MissingNativeStart);
            }
            if !known(conversation.as_deref()) {
                group
                    .incomplete_reasons
                    .push(CaptureGap::MissingConversationIdentity);
            }
            if !row.get::<_, bool>(4)? {
                group
                    .incomplete_reasons
                    .push(CaptureGap::DiscoveryIncomplete);
            }
            let stored_host: Option<String> = row.get(5)?;
            let stored_surface: Option<String> = row.get(6)?;
            if stored_host.as_deref().is_some_and(|h| h != group.host) {
                group.incomplete_reasons.push(CaptureGap::IdentityMismatch);
            }
            if stored_host.is_some() && stored_surface != group.surface {
                group.incomplete_reasons.push(CaptureGap::SurfaceMismatch);
            }
            if start.is_some()
                && known(conversation.as_deref())
                && known(group.surface.as_deref())
                && stored_host.as_deref() == Some(group.host.as_str())
                && stored_surface == group.surface
                && row.get::<_, bool>(7)?
            {
                group.captured_sessions += 1;
            }
        }
        for group in groups.values_mut() {
            if uncertain_hosts.contains(&group.host) {
                group
                    .incomplete_reasons
                    .push(CaptureGap::UnresolvedSurfaceDenominator);
            }
            group.incomplete_reasons.sort();
            group.incomplete_reasons.dedup();
            if group.incomplete_reasons.is_empty() {
                group.pct = pct(group.captured_sessions, group.observed_sessions);
            }
        }
        Ok(groups.into_values().collect())
    }
}
impl SurfaceCapture {
    fn new(key: CoverageSurface, inventory: InventoryState) -> Self {
        let mut incomplete_reasons = Vec::new();
        let reason = match inventory {
            InventoryState::FreshComplete => None,
            InventoryState::Unknown => Some(CaptureGap::InventoryUnknown),
            InventoryState::Incomplete => Some(CaptureGap::InventoryIncomplete),
            InventoryState::Stale => Some(CaptureGap::InventoryStale),
            InventoryState::MissingPython => Some(CaptureGap::MissingPython),
            InventoryState::Unavailable => Some(CaptureGap::StoreUnavailable),
        };
        incomplete_reasons.extend(reason);
        if !known(key.surface.as_deref()) {
            incomplete_reasons.push(CaptureGap::UnknownSurface);
        }
        Self {
            host: key.host,
            surface: key.surface,
            inventory,
            observed_sessions: 0,
            captured_sessions: 0,
            unknown_start_sessions: 0,
            pct: None,
            incomplete_reasons,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, functions::FunctionFlags};
    use xt_store::{Host, ingest::DiscoveredSession};
    #[test]
    fn coverage_snapshot_keeps_usage_gate_and_capture_consistent_during_commit() {
        let f = xt_fixtures::Fixture::load(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1"),
        )
        .unwrap();
        let mut db = f.build_db(false).unwrap();
        db.store_mut()
            .observe_discovered_session(&DiscoveredSession {
                host: Host::Claude,
                native_session_id: "native".into(),
                conversation_id: Some(f.sessions()[0].metadata.session_id.clone()),
                surface: Some("cli".into()),
                started_at_ms: Some(1788782400000),
                last_observed_at: 1788825600000,
                discovery_complete: true,
            })
            .unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let writer = Connection::open(db.path()).unwrap();
        writer.execute_batch("DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT session_id,host,surface,ts_ms,coverage_snapshot_write(ts) AS ts,uuid,type,is_human,text_len,human_is_eligible,human_text_len,human_excluded,role,tool_use_count,confirmed_automated_input FROM v_records WHERE ts_ms IS NOT NULL").unwrap();
        let path = db.path().to_owned();
        let written = std::sync::atomic::AtomicBool::new(false);
        metrics.connection.create_scalar_function("coverage_snapshot_write",1,FunctionFlags::SQLITE_UTF8,move |context| {
            if !written.swap(true,std::sync::atomic::Ordering::SeqCst) {Connection::open(&path)?.execute_batch("UPDATE usage SET output_tokens=NULL; UPDATE discovered_sessions SET started_at_ms=0;")?;}
            context.get::<String>(0)
        }).unwrap();
        let window = Window::new(1788220800000, 1788825600000).unwrap();
        let health = [DiscoveryHealth {
            host: "claude".into(),
            surface: Some("cli".into()),
            inventory: InventoryState::FreshComplete,
        }];
        let before = metrics.coverage(window, window.end_ms(), &health).unwrap();
        assert_eq!(
            (
                before.usage.total.measured,
                before.gate_14d.measured_sessions,
                before.capture[0].observed_sessions
            ),
            (1, 1, 1)
        );
        assert!(metrics.connection.is_autocommit());
        let transaction = metrics.connection.unchecked_transaction().unwrap();
        let after = metrics.coverage(window, window.end_ms(), &health).unwrap();
        assert_eq!(
            (
                after.usage.total.measured,
                after.gate_14d.measured_sessions,
                after.capture[0].observed_sessions
            ),
            (0, 0, 0)
        );
        assert!(!metrics.connection.is_autocommit());
        drop(transaction);
    }
}
