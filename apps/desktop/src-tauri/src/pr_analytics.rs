//! The PRs page's two local reads: the cached per-PR report for one preset and
//! confidence mode, and the exact linked sessions of one of its pull requests
//! over that report's own event window. Both are read-only compositions of the
//! core report and the Sessions page assembler; nothing here refreshes a pull
//! request, reads a source history or computes a metric of its own.
use crate::{
    dashboard::{checked_value, convert, dashboard_window, selected_window, validate_window},
    dto::{MetricClock, PrAnalyticsPage, SessionPage},
    state::StateError,
};
use jiff::tz::TimeZone;
use xt_metrics::{MetricsDb, Window};
use xt_store::pr_link::PrIdentity;

/// The first integer JavaScript cannot represent exactly.
const JSON_SAFE: u64 = 1 << 53;

/// The report over the preset window ending at the one captured `now_ms`, with
/// the token gate anchored to the same instant, read in one snapshot.
pub fn page(
    metrics: &MetricsDb,
    days: u32,
    now_ms: i64,
    zone: TimeZone,
    clock: MetricClock,
    confirmed_only: bool,
) -> Result<PrAnalyticsPage, StateError> {
    let window = selected_window(days, now_ms)?;
    let report = metrics.pr_analytics(window, now_ms, confirmed_only)?;
    let page = PrAnalyticsPage {
        window: dashboard_window(days, window, &zone, clock),
        report: convert(&report)?,
    };
    checked_value(&page)?;
    Ok(page)
}

/// A caller's linked-session request, as it crosses IPC.
#[derive(Clone, Copy, Debug)]
pub struct PrSessionsRequest<'a> {
    pub repository: &'a str,
    pub number: u64,
    pub confirmed_only: bool,
    pub window_days: u32,
    /// The displayed report's `window.end_ms`, never a new clock reading.
    pub window_end_ms: i64,
    /// The opaque cursor a previous page returned as `next`.
    pub after: Option<&'a str>,
}

/// A request that passed every check that needs no storage.
#[derive(Clone, Debug)]
pub struct PrSessions {
    pub identity: PrIdentity,
    pub confirmed_only: bool,
    pub days: u32,
    pub window: Window,
    pub after: Option<xt_store::session_list::SessionCursor>,
}

/// Check everything a request says before any database is opened: the preset,
/// the canonical identity, integers JavaScript sent exactly, the pinned window
/// and the cursor.
///
/// The window is the preset ending at the supplied anchor, the same window the
/// report selected when it captured that instant; it is not re-anchored to the
/// current clock, so every page of one drilldown measures the same window.
pub fn validate(request: PrSessionsRequest<'_>) -> Result<PrSessions, StateError> {
    validate_window(request.window_days)?;
    if request.number >= JSON_SAFE {
        return Err(StateError::InvalidPrSessions("pull request number"));
    }
    let identity = PrIdentity::from_parts(request.repository, request.number)
        .map_err(|_| StateError::InvalidPrSessions("pull request identity"))?;
    if request.window_end_ms.unsigned_abs() >= JSON_SAFE {
        return Err(StateError::InvalidPrSessions("window anchor"));
    }
    let window = selected_window(request.window_days, request.window_end_ms)
        .map_err(|_| StateError::InvalidPrSessions("window anchor"))?;
    Ok(PrSessions {
        identity,
        confirmed_only: request.confirmed_only,
        days: request.window_days,
        window,
        after: crate::dto::session_cursor(request.after)?,
    })
}

/// The indexed user sessions linked to exactly this pull request after the
/// confidence filter, one bounded page at a time, measured over the pinned
/// window. Membership is decided in SQL before the page bound and the cursor,
/// and is linked membership, not activity: a member with nothing in the window
/// is listed with its empty measurement. Rows, links and measurements come from
/// the Sessions page assembler in one read snapshot.
pub fn sessions(
    metrics: &MetricsDb,
    request: &PrSessions,
    zone: TimeZone,
    clock: MetricClock,
    catalog: &xt_metrics::PriceCatalog,
) -> Result<SessionPage, StateError> {
    crate::dto::filtered_page(
        metrics,
        request.days,
        request.window,
        zone,
        clock,
        &xt_store::session_list::SessionFilter {
            sort: Default::default(),
            pull_request: Some(xt_store::session_list::PrMembership {
                identity: &request.identity,
                confirmed_only: request.confirmed_only,
            }),
            ..Default::default()
        },
        request.after.as_ref(),
        catalog,
    )
}

/// The confidence modes every preset is exported in: all links, then only
/// exact and SHA links.
pub const CONFIDENCE_MODES: [bool; 2] = [false, true];

/// A fixture database's report for every preset and confidence mode, read by
/// the same assembler the command uses at the fixture's pinned instant.
pub fn fixture_pages(
    path: &std::path::Path,
    now_ms: i64,
) -> Result<Vec<PrAnalyticsPage>, StateError> {
    let metrics = MetricsDb::open(path)?;
    let mut pages = Vec::new();
    for days in crate::dashboard::WINDOW_PRESETS {
        for confirmed_only in CONFIDENCE_MODES {
            pages.push(page(
                &metrics,
                days,
                now_ms,
                TimeZone::UTC,
                MetricClock::Fixture,
                confirmed_only,
            )?);
        }
    }
    Ok(pages)
}

/// Each named pull request's linked sessions for every preset and confidence
/// mode, pinned to the fixture's own report end, through the same validation
/// and assembler as the command. A fixture holds one page of them; a longer
/// membership fails the export instead of leaving the preview a page it would
/// have to invent.
pub fn fixture_sessions(
    path: &std::path::Path,
    now_ms: i64,
    pull_requests: &[(String, u64)],
    prices: Option<&serde_json::Value>,
) -> Result<Vec<crate::dto::FixturePrSessions>, StateError> {
    let metrics = MetricsDb::open(path)?;
    let catalog = crate::dashboard::fixture_catalog(prices)?;
    let mut exported = Vec::new();
    for (repository, number) in pull_requests {
        for window_days in crate::dashboard::WINDOW_PRESETS {
            for confirmed_only in CONFIDENCE_MODES {
                let request = validate(PrSessionsRequest {
                    repository,
                    number: *number,
                    confirmed_only,
                    window_days,
                    window_end_ms: now_ms,
                    after: None,
                })?;
                let page = sessions(
                    &metrics,
                    &request,
                    TimeZone::UTC,
                    MetricClock::Fixture,
                    &catalog,
                )?;
                if page.next.is_some() {
                    return Err(StateError::FixtureInvalid);
                }
                exported.push(crate::dto::FixturePrSessions {
                    repository: request.identity.repository().to_owned(),
                    number: request.identity.number(),
                    confirmed_only,
                    window_days,
                    window_end_ms: now_ms,
                    page,
                });
            }
        }
    }
    Ok(exported)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>() -> PrSessionsRequest<'a> {
        PrSessionsRequest {
            repository: "example/atlas",
            number: 7,
            confirmed_only: true,
            window_days: 7,
            window_end_ms: 1_788_825_600_000,
            after: None,
        }
    }

    #[test]
    fn a_valid_request_pins_the_preset_to_its_anchor() {
        let valid = validate(PrSessionsRequest {
            repository: "Example/Atlas",
            ..request()
        })
        .unwrap();
        assert_eq!(valid.identity.repository(), "example/atlas");
        assert_eq!(valid.window.end_ms(), 1_788_825_600_000);
        assert_eq!(valid.window.start_ms(), 1_788_825_600_000 - 7 * 86_400_000);
        assert!(valid.after.is_none());
    }

    #[test]
    fn every_unsupported_part_is_refused_without_storage() {
        let refused = |request: PrSessionsRequest<'_>| validate(request).unwrap_err().to_string();
        for days in [0, 8, 15, 31] {
            assert_eq!(
                refused(PrSessionsRequest {
                    window_days: days,
                    ..request()
                }),
                "metric range must be 7, 14, or 30 days"
            );
        }
        for number in [0, JSON_SAFE, u64::MAX] {
            assert!(
                refused(PrSessionsRequest {
                    number,
                    ..request()
                })
                .starts_with("invalid pull-request session request"),
                "{number}"
            );
        }
        for repository in [
            "",
            "atlas",
            "example/atlas/pull",
            "example/atlas.git",
            " example/atlas",
            "https://github.com/example/atlas",
        ] {
            assert_eq!(
                refused(PrSessionsRequest {
                    repository,
                    ..request()
                }),
                "invalid pull-request session request: pull request identity",
                "{repository:?}"
            );
        }
        for anchor in [
            JSON_SAFE as i64,
            -(JSON_SAFE as i64),
            i64::MAX,
            i64::MIN,
            // A window that cannot be built at all.
            -(JSON_SAFE as i64) + 1,
        ] {
            assert_eq!(
                refused(PrSessionsRequest {
                    window_end_ms: anchor,
                    ..request()
                }),
                "invalid pull-request session request: window anchor",
                "{anchor}"
            );
        }
        for cursor in [
            "not json".to_owned(),
            "{\"time\":1}".to_owned(),
            "x".repeat(4097),
        ] {
            assert!(
                validate(PrSessionsRequest {
                    after: Some(&cursor),
                    ..request()
                })
                .is_err()
            );
        }
    }
}
