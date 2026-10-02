//! Thin Environment composition: M-17 counts stay in xt-metrics and registry
//! reading stays in xt-probes. This module only fixes the two windows, supplies
//! an unknown inventory, orders identity rows and converts to generated DTOs.
use crate::{
    dashboard::{checked_value, convert, dashboard_window, selected_window},
    dto::*,
    state::StateError,
};
use jiff::{Span, Timestamp, tz::TimeZone};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
use xt_metrics::{DayBucket, EnvUsage, HostInventory, Inventory, MetricsDb, Window};
use xt_probes::environment::{self as probe, EnvironmentProbe, ProbeHost, ProbeRoots};

/// The fixed strip: this many local calendar dates, whatever range is selected.
pub const STRIP_DAYS: u32 = 14;
/// The hosts supplied to M-17, each with an unknown inventory.
const HOSTS: [ProbeHost; 3] = [ProbeHost::Claude, ProbeHost::Codex, ProbeHost::Cursor];

/// Fourteen local calendar dates ending with the date that holds the last
/// instant of `[.., now)`: the current, possibly partial, local day. The
/// window starts at that first date's local midnight and ends at `now`.
pub fn strip_window(now_ms: i64, zone: &TimeZone) -> Result<Window, StateError> {
    let last = Timestamp::from_millisecond(now_ms - 1)
        .map_err(|_| StateError::InvalidMetricWindow)?
        .to_zoned(zone.clone());
    let start = last
        .start_of_day()
        .and_then(|day| day.checked_sub(Span::new().days(i64::from(STRIP_DAYS - 1))))
        .and_then(|day| day.start_of_day())
        .map_err(|_| StateError::InvalidMetricWindow)?;
    Ok(Window::new(start.timestamp().as_millisecond(), now_ms)?)
}

/// Every probed host is supplied as `Inventory::Unknown`. The registries
/// available here cannot prove a complete callable inventory, so the M-17 join
/// must never state installed-but-never-called or called-but-not-installed.
fn unknown_inventory() -> Vec<HostInventory> {
    HOSTS
        .iter()
        .map(|host| HostInventory {
            host: host.as_str().into(),
            installed: Inventory::Unknown,
        })
        .collect()
}

/// Per host and identity, summed over that host's surfaces, one counter per
/// reported day of that report's window.
type IdentityDays = BTreeMap<(String, MetricToolIdentity), Vec<u64>>;
fn per_identity(usage: &EnvUsage) -> Result<IdentityDays, StateError> {
    let mut rows = IdentityDays::new();
    for surface in &usage.observed {
        for identity in &surface.by_identity {
            let key = (surface.host.clone(), convert(&identity.identity)?);
            let days = rows
                .entry(key)
                .or_insert_with(|| vec![0; identity.by_day.len()]);
            for (total, day) in days.iter_mut().zip(&identity.by_day) {
                *total = total.checked_add(day.calls).ok_or(StateError::CountRange)?;
            }
        }
    }
    Ok(rows)
}

fn identity_rows(
    selected: &EnvUsage,
    strip: &EnvUsage,
    buckets: &[DayBucket],
) -> Result<Vec<EnvIdentityRow>, StateError> {
    let selected = per_identity(selected)?;
    let strip = per_identity(strip)?;
    let mut keys: Vec<_> = selected.keys().chain(strip.keys()).cloned().collect();
    keys.sort();
    keys.dedup();
    let mut rows: Vec<EnvIdentityRow> = keys
        .into_iter()
        .map(|key| {
            let calls = selected.get(&key).map_or(0, |days| days.iter().sum());
            let counts = strip
                .get(&key)
                .cloned()
                .unwrap_or_else(|| vec![0; buckets.len()]);
            let (host, identity) = key;
            EnvIdentityRow {
                order: 0,
                host,
                identity,
                calls,
                strip_calls: counts.iter().sum(),
                strip: buckets
                    .iter()
                    .zip(counts)
                    .map(|(bucket, calls)| MetricDayCalls {
                        date: bucket.date.to_string(),
                        start_ms: bucket.window.start_ms(),
                        end_ms: bucket.window.end_ms(),
                        calls,
                    })
                    .collect(),
            }
        })
        .collect();
    // Selected-range calls first, then strip calls, then host and identity:
    // a total order, so the top slice and the full list never depend on React.
    rows.sort_by(|a, b| {
        b.calls
            .cmp(&a.calls)
            .then_with(|| b.strip_calls.cmp(&a.strip_calls))
            .then_with(|| a.host.cmp(&b.host))
            .then_with(|| a.identity.cmp(&b.identity))
    });
    for (order, row) in rows.iter_mut().enumerate() {
        row.order = u32::try_from(order).map_err(|_| StateError::CountRange)?;
    }
    Ok(rows)
}

/// Compose the selected range and the fixed strip in one read snapshot, with
/// an unknown inventory, beside the supplied probe reading.
pub fn assemble(
    db: &MetricsDb,
    days: u32,
    now_ms: i64,
    zone: TimeZone,
    clock: MetricClock,
    probe: &EnvironmentProbe,
) -> Result<EnvironmentMetrics, StateError> {
    let window = selected_window(days, now_ms)?;
    let strip_window = strip_window(now_ms, &zone)?;
    let buckets = strip_window.local_days(zone.clone())?;
    if buckets.len() != STRIP_DAYS as usize {
        return Err(StateError::MetricEncoding);
    }
    let inventory = unknown_inventory();
    let (selected, strip) = db.read_snapshot(|db| {
        Ok((
            db.environment(window, zone.clone(), &inventory)?,
            db.environment(strip_window, zone.clone(), &inventory)?,
        ))
    })?;
    let identities = identity_rows(&selected, &strip, &buckets)?;
    let sum = |usage: &EnvUsage| usage.observed.iter().map(|surface| surface.calls).sum();
    let report = EnvironmentMetrics {
        window: dashboard_window(days, window, &zone, clock.clone()),
        strip_window: dashboard_window(STRIP_DAYS, strip_window, &zone, clock),
        inventory: MetricInventory::Unknown,
        totals: EnvTotals {
            selected_calls: sum(&selected),
            strip_calls: sum(&strip),
            selected_unresolved_calls: selected.unresolved.iter().map(|row| row.calls).sum(),
            identities: u32::try_from(identities.len()).map_err(|_| StateError::CountRange)?,
            configured_components: u32::try_from(probe.components.len())
                .map_err(|_| StateError::CountRange)?,
        },
        identities,
        selected: convert(&selected)?,
        strip: convert(&strip)?,
        configured: convert(&probe.components)?,
        sources: convert(&probe.sources)?,
        cache: convert(&probe.cache)?,
        roots: convert(&probe.roots)?,
    };
    checked_value(&report)?;
    Ok(report)
}

/// Native probe roots: the app's native home option plus local repository
/// paths stated by stored session metadata (the first session page, most
/// recent first). A remote identifier, relative value or traversal is dropped;
/// nothing is discovered, walked or asked of Git.
pub fn native_roots(home: Option<&Path>, stored: &[Option<String>]) -> ProbeRoots {
    let mut roots = ProbeRoots::new();
    if let Some(home) = home
        && let Ok(next) = roots.clone().with_home(home)
    {
        roots = next;
    }
    for value in stored.iter().flatten() {
        if let Ok(next) = roots.clone().with_repository(value) {
            roots = next;
        }
    }
    roots
}

/// The fixture probe: the real probe over F16's small synthetic matrix, with
/// roots taken from F16's declared `env` index. Fixture startup and the shell
/// exporter share it; it never reads a user's home.
pub fn fixture_probe(catalog: &Path) -> Result<EnvironmentProbe, StateError> {
    let base: PathBuf = catalog
        .canonicalize()
        .map_err(|_| StateError::FixtureInvalid)?
        .join("F16/input/probes");
    let text =
        std::fs::read_to_string(base.join("env.json")).map_err(|_| StateError::FixtureInvalid)?;
    let index: serde_json::Value =
        serde_json::from_str(&text).map_err(|_| StateError::FixtureInvalid)?;
    let root = |value: &serde_json::Value| {
        value
            .as_str()
            .filter(|path| {
                Path::new(path)
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
            })
            .map(|path| base.join(path))
            .ok_or(StateError::FixtureInvalid)
    };
    let mut roots = ProbeRoots::new()
        .with_home(root(&index["roots"]["home"])?)
        .map_err(|_| StateError::FixtureInvalid)?;
    for repository in index["roots"]["repositories"]
        .as_array()
        .ok_or(StateError::FixtureInvalid)?
    {
        let path = root(repository)?;
        roots = roots
            .with_repository(path.to_str().ok_or(StateError::FixtureInvalid)?)
            .map_err(|_| StateError::FixtureInvalid)?;
    }
    Ok(probe::probe(&roots))
}

/// Every preset over one fixture database with the fixture's pinned clock, UTC
/// and the shared fixture probe.
pub fn fixture_reports(
    path: &Path,
    now_ms: i64,
    catalog: &Path,
) -> Result<Vec<EnvironmentMetrics>, StateError> {
    let metrics = MetricsDb::open(path)?;
    let probe = fixture_probe(catalog)?;
    crate::dashboard::WINDOW_PRESETS
        .into_iter()
        .map(|days| {
            assemble(
                &metrics,
                days,
                now_ms,
                TimeZone::UTC,
                MetricClock::Fixture,
                &probe,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xt_probes::environment::{IncompleteReason, SourceStatus, UnsupportedReason};

    #[test]
    fn environment_conversion_rejects_unsafe_integers() {
        let day = |calls: u64| serde_json::json!({"date":"2026-09-07","start_ms":0,"end_ms":1,"calls":calls});
        let safe = (1_u64 << 53) - 1;
        assert_eq!(convert::<MetricDayCalls>(&day(safe)).unwrap().calls, safe);
        for unsafe_count in [1_u64 << 53, u64::MAX] {
            assert!(matches!(
                convert::<MetricDayCalls>(&day(unsafe_count)),
                Err(StateError::CountRange)
            ));
        }
    }

    /// Every probe status converts to the generated shape exactly, so a
    /// renamed core variant fails instead of reaching the UI as unknown.
    #[test]
    fn environment_probe_statuses_round_trip_through_the_generated_shape() {
        let statuses = [
            SourceStatus::Missing,
            SourceStatus::Empty,
            SourceStatus::Read { stated: 3 },
            SourceStatus::Incomplete {
                stated: 3,
                skipped: 1,
                reason: IncompleteReason::EntryLimit,
            },
            SourceStatus::Incomplete {
                stated: 3,
                skipped: 1,
                reason: IncompleteReason::UnprovenEntry,
            },
            SourceStatus::Malformed,
            SourceStatus::Unreadable,
            SourceStatus::Unsupported {
                reason: UnsupportedReason::Symlink,
            },
            SourceStatus::Unsupported {
                reason: UnsupportedReason::NotRegularFile,
            },
            SourceStatus::Unsupported {
                reason: UnsupportedReason::NotDirectory,
            },
            SourceStatus::Unsupported {
                reason: UnsupportedReason::FileTooLarge,
            },
            SourceStatus::Unsupported {
                reason: UnsupportedReason::UnsupportedSchema,
            },
        ];
        for status in statuses {
            convert::<EnvSourceStatus>(&status).unwrap();
        }
        assert!(convert::<EnvSourceStatus>(&serde_json::json!({"state":"absent"})).is_err());
    }

    #[test]
    fn environment_native_roots_drop_remote_relative_and_traversal_values() {
        let roots = native_roots(
            Some(Path::new("/home/user")),
            &[
                Some("/work/repo".into()),
                Some("https://github.com/o/r".into()),
                Some("o/r".into()),
                Some("/work/../etc".into()),
                None,
                Some("/work/repo".into()),
            ],
        );
        assert_eq!(roots.homes(), [PathBuf::from("/home/user")]);
        assert_eq!(roots.repositories(), [PathBuf::from("/work/repo")]);
        assert!(native_roots(None, &[]).homes().is_empty());
    }
}
