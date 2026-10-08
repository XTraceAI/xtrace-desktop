//! A report's current and previous windows, each window's selected responses
//! read once for every report that counts, prices or attributes them.
use crate::{
    CostReport, Coverage, DiscoveryHealth, FavoriteModel, MetricsDb, PriceCatalog, Result,
    TokenReport, TypingRate, Window, WindowActivity, cost, coverage::Cohort, favorite::Outputs,
    tokens,
};
use jiff::tz::TimeZone;
use xt_store::timestamp::InstantKey;

/// One window's reports that read its selected responses, each equal to its
/// own method over the same window and snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct PeriodReports {
    pub activity: WindowActivity,
    pub tokens: TokenReport,
    pub favorite: FavoriteModel,
    pub cost: CostReport,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ReportPeriods {
    pub current: PeriodReports,
    /// The equal-elapsed window before `current`.
    pub previous: PeriodReports,
    /// [`MetricsDb::coverage`] of the current window.
    pub coverage: Coverage,
}

/// The totals one window's token report, cost report, favorite model and,
/// for the selected window, coverage take from its selected responses. M-19's
/// cost read hands it each response it accepts, decoded as that read decodes
/// it; no response is kept.
pub(crate) struct UsageCollector<'a> {
    catalog: &'a PriceCatalog,
    tokens: tokens::Breakdowns,
    cost: cost::Breakdowns,
    outputs: Outputs,
    /// The window's event cohort, read before its responses.
    cohort: Option<&'a mut Cohort>,
}
impl UsageCollector<'_> {
    pub(crate) fn push(
        &mut self,
        instant: &InstantKey,
        session: &str,
        surface: Option<&str>,
        observation: &cost::Observation,
    ) -> Result<()> {
        let model = observation.raw_model();
        let counters = observation.counters();
        self.tokens.push(
            instant,
            session,
            observation.host(),
            model,
            surface,
            counters,
        )?;
        self.cost
            .push(instant, surface, observation, self.catalog)?;
        if let Some(output) = observation.output_tokens() {
            self.outputs.push(model, output)?;
        }
        if let Some(usage) = self.cohort.as_mut().and_then(|c| c.get_mut(session)) {
            usage.observe(model, counters.map(|c| c.is_some()));
        }
        Ok(())
    }
}

impl MetricsDb {
    /// [`PeriodReports`] for `window` and its previous window, and the
    /// current window's coverage, in one snapshot. Each window's responses
    /// are read once, by M-19's cost read, and folded as they are read into
    /// the other reports' totals; the current window's reports and coverage
    /// are finished before the previous window is read.
    #[allow(clippy::too_many_arguments)]
    pub fn report_periods(
        &self,
        window: Window,
        now_ms: i64,
        health: &[DiscoveryHealth],
        zone: TimeZone,
        typing_rate: TypingRate,
        confirmed_only: bool,
        catalog: &PriceCatalog,
    ) -> Result<ReportPeriods> {
        let previous = window.previous()?;
        self.read_snapshot(|db| {
            let mut cohort = db.cohort(window)?;
            let current = db.period_reports(
                window,
                zone.clone(),
                typing_rate,
                confirmed_only,
                catalog,
                Some(&mut cohort),
            )?;
            let coverage = db.coverage_from(window, now_ms, health, cohort)?;
            let previous =
                db.period_reports(previous, zone, typing_rate, confirmed_only, catalog, None)?;
            Ok(ReportPeriods {
                current,
                previous,
                coverage,
            })
        })
    }

    fn period_reports(
        &self,
        window: Window,
        zone: TimeZone,
        typing_rate: TypingRate,
        confirmed_only: bool,
        catalog: &PriceCatalog,
        cohort: Option<&mut Cohort>,
    ) -> Result<PeriodReports> {
        let mut shared = UsageCollector {
            catalog,
            tokens: tokens::Breakdowns::new(window, zone.clone())?,
            cost: cost::Breakdowns::new(window, zone.clone())?,
            outputs: Outputs::default(),
            cohort,
        };
        let activity = self.read_window_activity(
            window,
            zone,
            typing_rate,
            confirmed_only,
            catalog,
            Some(&mut shared),
        )?;
        // The responses' statement is closed: a favorite tie reads turns now.
        let UsageCollector {
            tokens,
            cost,
            outputs,
            ..
        } = shared;
        Ok(PeriodReports {
            activity,
            tokens: tokens.finish()?,
            favorite: self.favorite_from(window, outputs)?,
            cost: cost.finish(catalog),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, functions::FunctionFlags};
    use serde_json::json;
    use std::{
        path::Path,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };
    const DAY: i64 = 86_400_000;
    /// 2026-09-15T00:00:00Z.
    const NOW: i64 = 1789430400000;
    /// Three responses in the last 7 days, two in the 7 before, one more in
    /// the 14 before those; each after a human message.
    fn fixture() -> xt_fixtures::TempDb {
        let mut db = xt_fixtures::TempDb::empty().unwrap();
        let session = xt_store::SessionMeta::new("s", "claude", xt_store::SessionSource::Fixture);
        db.store_mut().upsert_session(&session, false).unwrap();
        let mut rows = Vec::new();
        for (i, day) in ["09-12", "09-13", "09-14", "09-03", "09-04", "08-20"]
            .into_iter()
            .enumerate()
        {
            rows.push(json!({"uuid":format!("h{i}"),"type":"user","timestamp":format!("2026-{day}T12:00:00Z"),"message":{"role":"user","content":[{"type":"text","text":"Synthetic"}]}}));
            rows.push(json!({"uuid":format!("a{i}"),"type":"assistant","timestamp":format!("2026-{day}T12:00:01Z"),"message":{"role":"assistant","model":"claude-haiku-4-5-20251001","content":[],"usage":{"input_tokens":1,"output_tokens":2,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"service_tier":"standard"}}}));
        }
        let rows: Vec<xt_store::CanonicalRecord> = serde_json::from_value(json!(rows)).unwrap();
        db.store_mut().upsert_records("s", &rows, false).unwrap();
        db
    }
    fn window(days: i64) -> Window {
        Window::new(NOW - days * DAY, NOW).unwrap()
    }
    fn shared(metrics: &MetricsDb, days: i64) -> ReportPeriods {
        metrics
            .report_periods(
                window(days),
                NOW,
                &[],
                TimeZone::UTC,
                TypingRate::default(),
                false,
                &PriceCatalog::bundled().unwrap(),
            )
            .unwrap()
    }
    fn standalone(metrics: &MetricsDb, days: i64) -> ReportPeriods {
        let catalog = PriceCatalog::bundled().unwrap();
        let period = |window: Window| PeriodReports {
            activity: metrics
                .window_activity(
                    window,
                    TimeZone::UTC,
                    TypingRate::default(),
                    false,
                    &catalog,
                )
                .unwrap(),
            tokens: metrics.tokens(window, TimeZone::UTC).unwrap(),
            favorite: metrics.favorite_model(window).unwrap(),
            cost: metrics.cost(window, TimeZone::UTC, &catalog).unwrap(),
        };
        ReportPeriods {
            current: period(window(days)),
            previous: period(window(days).previous().unwrap()),
            coverage: metrics.coverage(window(days), NOW, &[]).unwrap(),
        }
    }
    /// Every usage reader selects `ts`, so `on_row` runs once per usage row
    /// any of them reads. Only the pricing reads select `service_tier`, so
    /// `on_priced_row` runs once per row of those, with its `ts_ms`.
    fn watch_usage_rows(
        metrics: &MetricsDb,
        path: &Path,
        on_row: impl Fn() -> rusqlite::Result<()> + Send + 'static,
        on_priced_row: impl Fn(i64) + Send + 'static,
    ) {
        let writer = Connection::open(path).unwrap();
        let selected: String = writer
            .query_row(
                "SELECT sql FROM sqlite_master WHERE name='v_response_usage'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        writer.execute_batch(&format!(
            "{}; DROP VIEW v_response_usage; CREATE VIEW v_response_usage AS SELECT session_id,host,model,surface,usage_row(ts) AS ts,ts_ms,input_tokens,output_tokens,cache_read_tokens,cache_creation_tokens,cache_creation_5m,cache_creation_1h,priced_row(service_tier,ts_ms) AS service_tier FROM zz_selected",
            selected.replacen("CREATE VIEW v_response_usage", "CREATE VIEW zz_selected", 1)
        )).unwrap();
        metrics
            .connection
            .create_scalar_function("usage_row", 1, FunctionFlags::SQLITE_UTF8, move |context| {
                on_row()?;
                context.get::<String>(0)
            })
            .unwrap();
        metrics
            .connection
            .create_scalar_function(
                "priced_row",
                2,
                FunctionFlags::SQLITE_UTF8,
                move |context| {
                    on_priced_row(context.get(1)?);
                    context.get::<Option<String>>(0)
                },
            )
            .unwrap();
    }

    #[test]
    fn report_periods_read_each_windows_responses_once() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let rows = Arc::new(AtomicUsize::new(0));
        let counted = rows.clone();
        watch_usage_rows(
            &metrics,
            db.path(),
            move || {
                counted.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_| {},
        );
        let mut one_read = |window: Window| {
            metrics.tokens(window, TimeZone::UTC).unwrap();
            rows.swap(0, Ordering::SeqCst)
        };
        let fixed = window(14);
        assert_eq!(
            [7, 14].map(|days| [window(days), window(days).previous().unwrap()].map(&mut one_read)),
            [[3, 2], [5, 1]]
        );
        assert_eq!(one_read(fixed), 5);
        // 7 days: the gate's fixed 14 days are another window, read on their own.
        assert_eq!(standalone(&metrics, 7), shared(&metrics, 7));
        assert_eq!(
            rows.swap(0, Ordering::SeqCst),
            4 * 3 + 4 * 2 + 3 + 5 + 3 + 2 + 5
        );
        // 14 days: the selected window is the gate's, so its responses serve both.
        assert_eq!(standalone(&metrics, 14), shared(&metrics, 14));
        assert_eq!(rows.swap(0, Ordering::SeqCst), 4 * 5 + 4 + 5 + 5 + 1);
    }

    #[test]
    fn report_periods_read_the_current_window_before_the_previous() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        watch_usage_rows(
            &metrics,
            db.path(),
            || Ok(()),
            move |ts_ms| record.lock().unwrap().push(ts_ms),
        );
        for (days, current, previous) in [(7, 3, 2), (14, 5, 1)] {
            let report = shared(&metrics, days);
            assert_eq!(report.current.tokens.total.selected_responses, current);
            assert_eq!(report.previous.tokens.total.selected_responses, previous);
            // Each priced row once: all of the current window's, then the
            // previous window's.
            let start = window(days).start_ms();
            let seen = std::mem::take(&mut *seen.lock().unwrap());
            let current_rows = seen.iter().take_while(|ts_ms| **ts_ms >= start).count();
            assert_eq!(current_rows, current as usize);
            assert_eq!(seen.len(), (current + previous) as usize);
            assert!(seen[current_rows..].iter().all(|ts_ms| *ts_ms < start));
        }
    }

    #[test]
    fn report_periods_hold_through_a_writer_commit_then_reread() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let armed = Arc::new(AtomicBool::new(false));
        let trigger = armed.clone();
        let path = db.path().to_owned();
        // The commit lands on the current window's first usage row, after its
        // event cohort, spans, human time and links were read. The rest of
        // that read, M-19's later reads, coverage's gate and capture and the
        // whole previous window come after it, and see the earlier state only
        // because they share one snapshot.
        watch_usage_rows(
            &metrics,
            db.path(),
            move || {
                if trigger.swap(false, Ordering::SeqCst) {
                    Connection::open(&path)?.execute_batch(
                        "UPDATE usage SET output_tokens=NULL; UPDATE records SET is_human=0;",
                    )?;
                }
                Ok(())
            },
            |_| {},
        );
        let before = standalone(&metrics, 7);
        armed.store(true, Ordering::SeqCst);
        assert_eq!(shared(&metrics, 7), before);
        assert!(!armed.load(Ordering::SeqCst));
        assert!(metrics.connection.is_autocommit());
        let after = shared(&metrics, 7);
        assert_eq!(after, standalone(&metrics, 7));
        assert_ne!(after.current.tokens, before.current.tokens);
        assert_ne!(after.previous.favorite, before.previous.favorite);
        assert_ne!(after.coverage.usage, before.coverage.usage);
        assert_ne!(after.current.activity.human, before.current.activity.human);
        assert_ne!(
            after.previous.activity.pr_effort,
            before.previous.activity.pr_effort
        );
    }

    #[test]
    fn report_periods_leave_a_caller_snapshot_open() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let transaction = metrics.connection.unchecked_transaction().unwrap();
        let before = shared(&metrics, 14);
        Connection::open(db.path())
            .unwrap()
            .execute_batch("UPDATE usage SET output_tokens=NULL; UPDATE records SET is_human=0;")
            .unwrap();
        assert_eq!(shared(&metrics, 14), before);
        assert!(!metrics.connection.is_autocommit());
        transaction.commit().unwrap();
        let after = shared(&metrics, 14);
        assert_ne!(after.current.cost, before.current.cost);
        assert_ne!(after.coverage.gate_14d, before.coverage.gate_14d);
    }
}
