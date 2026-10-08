//! Shared by the metric tests that check `MetricsDb::report_periods` against
//! the standalone reports it combines, on each test's own fixture.
#![allow(dead_code)]
use jiff::tz::TimeZone;
use xt_metrics::{
    CostReport, Coverage, DiscoveryHealth, FavoriteModel, MetricsDb, PeriodReports, PriceCatalog,
    ReportPeriods, Result, TokenReport, TypingRate, Window, WindowActivity,
};

/// One window's standalone reports, each from its own method.
pub struct Period {
    pub activity: Result<WindowActivity>,
    pub tokens: Result<TokenReport>,
    pub favorite: Result<FavoriteModel>,
    pub cost: Result<CostReport>,
}
pub struct Standalone {
    pub current: Period,
    pub previous: Period,
    pub coverage: Result<Coverage>,
}
impl Period {
    fn read(
        metrics: &MetricsDb,
        window: Window,
        zone: TimeZone,
        confirmed_only: bool,
        catalog: &PriceCatalog,
    ) -> Self {
        Self {
            activity: metrics.window_activity(
                window,
                zone.clone(),
                TypingRate::default(),
                confirmed_only,
                catalog,
            ),
            tokens: metrics.tokens(window, zone.clone()),
            favorite: metrics.favorite_model(window),
            cost: metrics.cost(window, zone, catalog),
        }
    }
    fn failed(&self) -> bool {
        self.activity.is_err()
            || self.tokens.is_err()
            || self.favorite.is_err()
            || self.cost.is_err()
    }
    fn reports(&self) -> PeriodReports {
        PeriodReports {
            activity: self.activity.as_ref().unwrap().clone(),
            tokens: self.tokens.as_ref().unwrap().clone(),
            favorite: self.favorite.as_ref().unwrap().clone(),
            cost: self.cost.as_ref().unwrap().clone(),
        }
    }
}

/// The combined read agrees with every standalone method over the same
/// window, its previous window and coverage: the same reports when they all
/// succeed, and an error exactly when at least one of them fails. Returns
/// the standalone results.
pub fn matches_standalone(
    metrics: &MetricsDb,
    window: Window,
    now_ms: i64,
    health: &[DiscoveryHealth],
    zone: TimeZone,
    catalog: &PriceCatalog,
) -> Standalone {
    matches_standalone_links(metrics, window, now_ms, health, zone, false, catalog)
}
/// [`matches_standalone`] with M-19's link filter.
pub fn matches_standalone_links(
    metrics: &MetricsDb,
    window: Window,
    now_ms: i64,
    health: &[DiscoveryHealth],
    zone: TimeZone,
    confirmed_only: bool,
    catalog: &PriceCatalog,
) -> Standalone {
    let standalone = Standalone {
        current: Period::read(metrics, window, zone.clone(), confirmed_only, catalog),
        previous: Period::read(
            metrics,
            window.previous().unwrap(),
            zone.clone(),
            confirmed_only,
            catalog,
        ),
        coverage: metrics.coverage(window, now_ms, health),
    };
    let failed =
        standalone.current.failed() || standalone.previous.failed() || standalone.coverage.is_err();
    match metrics.report_periods(
        window,
        now_ms,
        health,
        zone,
        TypingRate::default(),
        confirmed_only,
        catalog,
    ) {
        Ok(shared) => {
            assert!(!failed, "the combined read hid a standalone failure");
            assert_eq!(
                shared,
                ReportPeriods {
                    current: standalone.current.reports(),
                    previous: standalone.previous.reports(),
                    coverage: standalone.coverage.as_ref().unwrap().clone(),
                }
            );
        }
        Err(error) => assert!(
            failed,
            "the combined read failed where every standalone read succeeded: {error}"
        ),
    }
    standalone
}
