use crate::{
    ActiveSpanReport, Concurrency, HumanTime, MetricsDb, PrEffortReport, PriceCatalog, Result,
    TypingRate, Window, sweep::sweep,
};
use jiff::tz::TimeZone;

/// The four reports one window's M-05 spans feed, each equal to its own
/// method over the same window and snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct WindowActivity {
    pub spans: ActiveSpanReport,
    pub human: HumanTime,
    pub concurrency: Concurrency,
    pub pr_effort: PrEffortReport,
}

impl MetricsDb {
    /// [`MetricsDb::active_spans`], [`MetricsDb::human_time`],
    /// [`MetricsDb::concurrency`] and [`MetricsDb::pr_effort`] over one
    /// window in one snapshot, reading the spans once for all four.
    pub fn window_activity(
        &self,
        window: Window,
        zone: TimeZone,
        typing_rate: TypingRate,
        confirmed_only: bool,
        catalog: &PriceCatalog,
    ) -> Result<WindowActivity> {
        self.read_snapshot(|db| {
            let spans = db.active_spans(window)?;
            Ok(WindowActivity {
                human: db.read_human_time(window, typing_rate, zone.clone(), Some(&spans))?,
                concurrency: sweep(&spans.spans)?,
                pr_effort: db.read_pr_effort(
                    window,
                    zone,
                    confirmed_only,
                    catalog,
                    Some(&spans),
                )?,
                spans,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, functions::FunctionFlags};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    fn fixture() -> xt_fixtures::TempDb {
        xt_fixtures::Fixture::load(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/F1"),
        )
        .unwrap()
        .build_db(false)
        .unwrap()
    }
    fn window() -> Window {
        Window::new(1788220800000, 1788825600000).unwrap()
    }
    fn activity(metrics: &MetricsDb) -> WindowActivity {
        metrics
            .window_activity(
                window(),
                TimeZone::UTC,
                TypingRate::default(),
                false,
                &PriceCatalog::bundled().unwrap(),
            )
            .unwrap()
    }
    fn standalone(metrics: &MetricsDb) -> WindowActivity {
        let catalog = PriceCatalog::bundled().unwrap();
        WindowActivity {
            spans: metrics.active_spans(window()).unwrap(),
            human: metrics
                .human_time(window(), TypingRate::default(), TimeZone::UTC)
                .unwrap(),
            concurrency: metrics.concurrency(window()).unwrap(),
            pr_effort: metrics
                .pr_effort(window(), TimeZone::UTC, false, &catalog)
                .unwrap(),
        }
    }
    /// Only the span read selects `uuid` from the event view among these
    /// four reports, so each call of `on_span_row` is one span-read row.
    fn watch_span_rows(
        metrics: &MetricsDb,
        path: &std::path::Path,
        on_span_row: impl Fn() -> rusqlite::Result<()> + Send + 'static,
    ) {
        Connection::open(path).unwrap().execute_batch("DROP VIEW v_session_events; CREATE VIEW v_session_events AS SELECT session_id,host,surface,ts_ms,ts,span_row(uuid) AS uuid,type,is_human,text_len,human_is_eligible,human_text_len,human_excluded,role,tool_use_count,confirmed_automated_input FROM v_records WHERE ts_ms IS NOT NULL").unwrap();
        metrics
            .connection
            .create_scalar_function("span_row", 1, FunctionFlags::SQLITE_UTF8, move |context| {
                on_span_row()?;
                context.get::<String>(0)
            })
            .unwrap();
    }

    #[test]
    fn window_activity_reads_the_spans_once_for_all_four_reports() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let rows = Arc::new(AtomicUsize::new(0));
        let counted = rows.clone();
        watch_span_rows(&metrics, db.path(), move || {
            counted.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        metrics.active_spans(window()).unwrap();
        let one_read = rows.swap(0, Ordering::SeqCst);
        assert!(one_read > 0);
        let shared = activity(&metrics);
        assert_eq!(rows.swap(0, Ordering::SeqCst), one_read);
        // Standalone, human time, concurrency and M-19 each read the spans.
        assert_eq!(standalone(&metrics), shared);
        assert_eq!(rows.load(Ordering::SeqCst), 4 * one_read);
    }

    #[test]
    fn window_activity_snapshot_holds_through_a_writer_commit_then_rereads() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let armed = Arc::new(AtomicBool::new(false));
        let trigger = armed.clone();
        let path = db.path().to_owned();
        // The spans are read first, so a commit landing on the first span row
        // would reach the human, cost and M-19 reads after it if the four
        // reports were not one snapshot.
        watch_span_rows(&metrics, db.path(), move || {
            if trigger.swap(false, Ordering::SeqCst) {
                Connection::open(&path)?
                    .execute_batch("UPDATE records SET is_human=0; DELETE FROM pr_links;")?;
            }
            Ok(())
        });
        let before = standalone(&metrics);
        assert_eq!(before.human.human_minutes_est, Some(0.675));
        armed.store(true, Ordering::SeqCst);
        assert_eq!(activity(&metrics), before);
        assert!(!armed.load(Ordering::SeqCst));
        assert!(metrics.connection.is_autocommit());
        let after = activity(&metrics);
        assert_eq!(after, standalone(&metrics));
        assert_eq!(after.human.human_minutes_est, Some(0.0));
        assert_ne!(after.pr_effort, before.pr_effort);
    }

    #[test]
    fn window_activity_leaves_a_caller_snapshot_open() {
        let db = fixture();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let transaction = metrics.connection.unchecked_transaction().unwrap();
        let before = activity(&metrics);
        Connection::open(db.path())
            .unwrap()
            .execute_batch("UPDATE records SET is_human=0; DELETE FROM pr_links;")
            .unwrap();
        assert_eq!(activity(&metrics), before);
        assert!(!metrics.connection.is_autocommit());
        transaction.commit().unwrap();
        let after = activity(&metrics);
        assert_eq!(after.human.human_minutes_est, Some(0.0));
        assert_ne!(after.pr_effort, before.pr_effort);
    }
}
