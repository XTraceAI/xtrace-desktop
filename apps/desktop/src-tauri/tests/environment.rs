//! The Environment bridge: two disclosed windows over one snapshot, an unknown
//! inventory, configured components and source statuses beside the counts, and
//! nothing private in the transport.
use jiff::{Timestamp, tz::TimeZone};
use serde_json::json;
use std::path::Path;
use xt_store::{
    CanonicalRecord, SessionMeta, SessionSource, Store,
    ingest::{ToolEvent, ToolKind},
    tool_use::HOOK_EVENT_NAME,
};
use xtrace_desktop::{
    dto::*,
    environment::{STRIP_DAYS, fixture_probe, strip_window},
    state::{AppState, StartupOptions, StateError},
};

const DAY_MS: i64 = 86_400_000;

#[test]
fn environment_native_home_spellings_share_startup_and_probe_rules() {
    let guard = tempfile::TempDir::new().unwrap();
    let root = guard.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("home")).unwrap();
    std::fs::create_dir_all(root.join("child-cwd")).unwrap();
    std::fs::write(
        root.join("home/.claude.json"),
        r#"{"mcpServers":{"native-home-server":{}}}"#,
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("home"), root.join("home-link")).unwrap();
    let mut cases = vec![
        "relative",
        "dot",
        "absolute",
        "default-relative",
        "default-absolute",
        "empty",
        "default-empty",
        "missing",
        "parent",
        "destination",
        "parent-destination",
    ];
    #[cfg(unix)]
    cases.push("symlink");
    let mut failures = Vec::new();
    for case in cases {
        let cwd = if case.starts_with("parent") {
            root.join("child-cwd")
        } else {
            root.clone()
        };
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "environment_native_home_spelling_child",
                "--nocapture",
            ])
            .current_dir(cwd)
            .env("XTRACE_TEST_NATIVE_HOME_CASE", case)
            .output()
            .unwrap();
        if !output.status.success() {
            failures.push(format!(
                "{case}:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
#[ignore = "run only by the parent spelling test in a controlled child cwd"]
fn environment_native_home_spelling_child() {
    let case = std::env::var("XTRACE_TEST_NATIVE_HOME_CASE").unwrap();
    let cwd = std::env::current_dir().unwrap();
    let selected = match case.as_str() {
        "relative" | "destination" => Some(std::path::PathBuf::from("home")),
        "dot" => Some(std::path::PathBuf::from("./home")),
        "absolute" => Some(cwd.join("./home")),
        "default-relative" | "default-absolute" | "default-empty" => None,
        "empty" => Some(std::path::PathBuf::new()),
        "missing" => Some(std::path::PathBuf::from("missing-home")),
        "parent" | "parent-destination" => Some(std::path::PathBuf::from("../home")),
        "symlink" => Some(cwd.join("home-link")),
        _ => panic!("unknown child case"),
    };
    let data = match case.as_str() {
        "destination" => cwd.join("home/.claude/index-data"),
        "parent-destination" => cwd.join("../home/.claude/index-data"),
        _ => cwd.join(format!("data-{case}")),
    };
    assert!(!data.exists());
    let default_calls = std::cell::Cell::new(0);
    let result = AppState::build(
        StartupOptions {
            data_dir: Some(data.clone()),
            native_home: selected.clone(),
            ..Default::default()
        },
        || panic!("explicit synthetic data directory"),
        || {
            default_calls.set(default_calls.get() + 1);
            match case.as_str() {
                "default-relative" => Ok(std::path::PathBuf::from("home")),
                "default-absolute" => Ok(cwd.join("home")),
                "default-empty" => Ok(std::path::PathBuf::new()),
                _ => panic!("explicit home must avoid defaults"),
            }
        },
    );
    assert_eq!(
        default_calls.get(),
        usize::from(case.starts_with("default"))
    );
    if matches!(case.as_str(), "empty" | "default-empty" | "missing") {
        assert!(matches!(result, Err(StateError::NativeHome)));
        assert!(!data.exists(), "invalid home must not create storage");
        return;
    }
    if case.ends_with("destination") {
        assert!(matches!(result, Err(StateError::IndexDestination(_))));
        assert!(!data.exists(), "unsafe destination must not create storage");
        return;
    }
    let state = result.unwrap();
    assert!(
        state.native_home().unwrap().is_dir(),
        "startup accepted an existing synthetic home"
    );
    let report = state.metrics_environment(7).unwrap();
    match case.as_str() {
        "parent" => {
            assert!(report.roots.is_empty());
            assert!(
                report.configured.is_empty(),
                "parent-containing homes remain refused by the probe"
            );
        }
        "symlink" => {
            assert_eq!(report.roots.len(), 1);
            assert_eq!(report.roots[0].state, EnvRootState::Symlink);
            assert!(
                report.configured.is_empty(),
                "symlink homes remain refused by the probe"
            );
        }
        _ => {
            assert_eq!(
                report
                    .configured
                    .iter()
                    .map(|component| component.name.as_str())
                    .collect::<Vec<_>>(),
                ["native-home-server"],
                "startup accepted {case} home but Environment omitted its configuration; roots={:?}",
                report.roots
            );
            assert_eq!(report.roots.len(), 1);
            assert_eq!(report.roots[0].state, EnvRootState::Read);
        }
    }
    let expected = match case.as_str() {
        "absolute" | "symlink" => selected.unwrap(),
        "parent" => cwd.join("../home"),
        _ => cwd.join("home"),
    };
    // The native index receives this same state field; compare raw spelling
    // rather than canonicalizing links or collapsing parent components.
    assert_eq!(
        state.native_home().unwrap().as_os_str(),
        expected.as_os_str()
    );
    state.shutdown();
}

fn ms(text: &str) -> i64 {
    text.parse::<Timestamp>().unwrap().as_millisecond()
}

fn tool_call(uuid: &str, timestamp: &str, name: &str) -> CanonicalRecord {
    serde_json::from_value(json!({"uuid":uuid,"type":"assistant","timestamp":timestamp,
        "message":{"role":"assistant","model":"test-claude","content":[
            {"type":"tool_use","id":format!("{uuid}-0"),"name":name,"input":{}}]}}))
    .unwrap()
}

fn session(
    store: &mut Store,
    id: &str,
    surface: Option<&str>,
    cwd: Option<&str>,
    rows: &[CanonicalRecord],
) {
    let mut meta = SessionMeta::new(id, "claude", SessionSource::Fixture);
    meta.surface = surface.map(str::to_owned);
    meta.cwd = cwd.map(str::to_owned);
    store.upsert_session(&meta, false).unwrap();
    store.upsert_records(id, rows, false).unwrap();
}

fn assert_strip(report: &EnvironmentMetrics) {
    let strip = &report.strip_window;
    assert_eq!(strip.days, STRIP_DAYS);
    assert_eq!(
        strip.end_ms, report.window.end_ms,
        "same clock anchors both"
    );
    assert_eq!(
        (strip.timezone.as_str(), &strip.clock),
        (report.window.timezone.as_str(), &report.window.clock)
    );
    assert_eq!(
        (report.strip.window_start_ms, report.strip.window_end_ms),
        (strip.start_ms, strip.end_ms)
    );
    for row in &report.identities {
        assert_eq!(row.strip.len(), STRIP_DAYS as usize);
        assert_eq!(row.strip[0].start_ms, strip.start_ms);
        assert_eq!(row.strip.last().unwrap().end_ms, strip.end_ms);
        assert_eq!(
            row.strip.iter().map(|day| day.calls).sum::<u64>(),
            row.strip_calls
        );
    }
}

/// Exactly 14 local calendar dates ending in the current partial local day,
/// at midnight, mid-day and across a DST change.
#[test]
fn environment_strip_is_exactly_fourteen_local_dates_ending_in_the_current_day() {
    // Mid-day: the last bucket is today's partial day.
    let now = ms("2026-09-07T15:30:00Z");
    let window = strip_window(now, &TimeZone::UTC).unwrap();
    let days = window.local_days(TimeZone::UTC).unwrap();
    assert_eq!(days.len(), 14);
    assert_eq!(days[0].date.to_string(), "2026-08-25");
    assert_eq!(days[13].date.to_string(), "2026-09-07");
    assert_eq!(
        (days[13].window.start_ms(), days[13].window.end_ms()),
        (ms("2026-09-07T00:00:00Z"), now)
    );
    // At local midnight [.., now) ends with the previous date, still 14 dates.
    let midnight = ms("2026-09-08T00:00:00Z");
    let days = strip_window(midnight, &TimeZone::UTC)
        .unwrap()
        .local_days(TimeZone::UTC)
        .unwrap();
    assert_eq!(
        (days.len(), days[13].date.to_string()),
        (14, "2026-09-07".into())
    );
    // Across the November DST change: 14 dates, one of them 25 hours long.
    let zone = TimeZone::get("America/Los_Angeles").unwrap();
    let now = ms("2026-11-05T12:00:00-08:00");
    let days = strip_window(now, &zone)
        .unwrap()
        .local_days(zone.clone())
        .unwrap();
    assert_eq!(days.len(), 14);
    assert_eq!(days[0].date.to_string(), "2026-10-23");
    let lengths: Vec<i64> = days
        .iter()
        .map(|day| day.window.end_ms() - day.window.start_ms())
        .collect();
    assert_eq!(
        lengths.iter().filter(|len| **len == 25 * 3_600_000).count(),
        1
    );
    assert_eq!(*lengths.last().unwrap(), 12 * 3_600_000);
}

fn native_state(root: &Path) -> AppState {
    AppState::build(
        StartupOptions {
            data_dir: Some(root.join("data")),
            native_home: Some(root.join("home")),
            ..Default::default()
        },
        || panic!("explicit data directory"),
        || panic!("explicit native home"),
    )
    .unwrap()
}

#[test]
fn environment_repository_roots_stop_at_the_first_visible_session_page() {
    let guard = tempfile::TempDir::new().unwrap();
    let root = guard.path().canonicalize().unwrap();
    let home = root.join("home");
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    std::fs::write(
        home.join(".cursor/mcp.json"),
        r#"{"mcpServers":{"home-retained":{}}}"#,
    )
    .unwrap();
    let repositories = [
        root.join("shared-repository"),
        root.join("visible-boundary-repository"),
        root.join("lookahead-repository"),
    ];
    for (repository, name) in repositories.iter().zip([
        "shared-repository-server",
        "visible-boundary-server",
        "lookahead-server",
    ]) {
        std::fs::create_dir_all(repository).unwrap();
        std::fs::write(
            repository.join(".mcp.json"),
            serde_json::to_vec(&json!({"mcpServers": {name: {}}})).unwrap(),
        )
        .unwrap();
    }
    let state = native_state(&root);
    let mut writer = Store::open(root.join("data/xtrace.db")).unwrap();
    for position in 0..51 {
        let repository = match position {
            49 => &repositories[1],
            50 => &repositories[2],
            _ => &repositories[0],
        };
        let mut metadata = SessionMeta::new(
            format!("synthetic-page-{position:02}"),
            "claude",
            SessionSource::Fixture,
        );
        // Explicit descending times determine positions independently of IDs.
        // Cwd supplies the same repo fallback the visible Sessions page uses.
        metadata.started_at_ms = Some(1000 - i64::from(position));
        metadata.cwd = Some(repository.to_str().unwrap().to_owned());
        writer.upsert_session(&metadata, false).unwrap();
    }
    let raw = writer.sessions_page("", None, None).unwrap();
    assert_eq!(raw.len(), 51);
    assert_eq!(raw[49].id, "synthetic-page-49");
    assert_eq!(raw[50].id, "synthetic-page-50");
    assert_eq!(raw[49].repo.as_deref(), repositories[1].to_str());
    assert_eq!(raw[50].repo.as_deref(), repositories[2].to_str());

    let page = state.sessions_list("", None, None).unwrap();
    assert_eq!(page.rows.len(), 50);
    assert!(page.next.is_some());
    assert_eq!(page.rows.last().unwrap().id, "synthetic-page-49");
    assert!(page.rows.iter().all(|row| row.id != "synthetic-page-50"));

    let report = state.metrics_environment(7).unwrap();
    assert_eq!(
        report
            .configured
            .iter()
            .map(|component| (component.host.as_str(), component.name.as_str()))
            .collect::<Vec<_>>(),
        [
            ("claude", "shared-repository-server"),
            ("claude", "visible-boundary-server"),
            ("cursor", "home-retained"),
        ]
    );
    assert_eq!(
        report
            .roots
            .iter()
            .map(|row| (row.scope, row.index, row.state))
            .collect::<Vec<_>>(),
        [
            (EnvRootScope::Home, 0, EnvRootState::Read),
            (EnvRootScope::Repository, 0, EnvRootState::Read),
            (EnvRootScope::Repository, 1, EnvRootState::Read),
        ]
    );
    assert_eq!(report.totals.configured_components, 3);
}

/// The native command probes the native home option and the local repository
/// paths stored sessions state, keeps inventory unknown for every host, keeps
/// hook summaries unresolved, preserves raw surfaces, rereads on every call
/// and never transports a path.
#[test]
fn environment_native_state_composes_counts_unknown_inventory_and_configured_facts() {
    let guard = tempfile::TempDir::new().unwrap();
    // The probe refuses a root under a linked ancestor (macOS `/var`); the
    // test resolves its temporary root once, the probe never does.
    let root = guard.path().canonicalize().unwrap();
    let home = root.as_path().join("home");
    let repo = root.as_path().join("work/repo");
    std::fs::create_dir_all(home.join(".cursor")).unwrap();
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        home.join(".cursor/mcp.json"),
        r#"{"mcpServers":{"alpha":{"command":"/secret/bin/alpha","env":{"K":"V"}}}}"#,
    )
    .unwrap();
    std::fs::write(repo.join(".mcp.json"), r#"{"mcpServers":{"beta":{}}}"#).unwrap();
    let state = native_state(root.as_path());
    let empty = state.metrics_environment(7).unwrap();
    assert_eq!(empty.window.clock, MetricClock::System);
    assert_eq!(empty.totals.selected_calls, 0);
    assert!(empty.identities.is_empty());
    assert_eq!(
        empty
            .configured
            .iter()
            .map(|c| (c.host.as_str(), c.name.as_str()))
            .collect::<Vec<_>>(),
        [("cursor", "alpha")],
        "no session states a repository yet"
    );

    let now = Timestamp::now().as_second();
    let at = |offset: i64| Timestamp::from_second(now + offset).unwrap().to_string();
    let mut writer = Store::open(root.as_path().join("data/xtrace.db")).unwrap();
    session(
        &mut writer,
        "local",
        Some("cli"),
        Some(repo.to_str().unwrap()),
        &[tool_call("a", &at(-7200), "Read")],
    );
    session(
        &mut writer,
        "remote",
        Some("future.app"),
        Some("https://github.com/XTraceAI/xtrace-desktop"),
        &[tool_call("b", &at(-3600), "mcp__memhub__search")],
    );
    session(&mut writer, "legacy", None, Some("relative/repo"), &[]);
    writer
        .insert_tool_event(&ToolEvent {
            session_id: "local".into(),
            source: SessionSource::Fixture,
            source_event_id: "hook-1".into(),
            timestamp: Some(at(-1800)),
            name: HOOK_EVENT_NAME.into(),
            kind: ToolKind::Hook,
            server: None,
            tool: None,
            skill: None,
        })
        .unwrap();

    let mut windows = Vec::new();
    for days in [7_u32, 14, 30] {
        let report = state.metrics_environment(days).unwrap();
        assert_eq!(report.window.days, days);
        assert_eq!(
            report.window.end_ms - report.window.start_ms,
            i64::from(days) * DAY_MS
        );
        assert_strip(&report);
        windows.push(report.strip_window.start_ms);
        // The inventory is unknown, always, for every host.
        assert_eq!(report.inventory, MetricInventory::Unknown);
        for usage in [&report.selected, &report.strip] {
            assert_eq!(
                usage
                    .hosts
                    .iter()
                    .map(|h| (h.host.as_str(), &h.inventory))
                    .collect::<Vec<_>>(),
                [
                    ("claude", &MetricInventoryJoin::Unknown),
                    ("codex", &MetricInventoryJoin::Unknown),
                    ("cursor", &MetricInventoryJoin::Unknown),
                ]
            );
        }
        // Raw surfaces stay verbatim; the summary is counted and unresolved.
        assert_eq!(
            report
                .selected
                .observed
                .iter()
                .map(|s| (s.surface.as_deref(), s.calls))
                .collect::<Vec<_>>(),
            [(Some("cli"), 2), (Some("future.app"), 1)]
        );
        assert_eq!(
            report.selected.unresolved,
            [MetricUnresolvedCalls {
                host: "claude".into(),
                reason: MetricUnresolvedReason::HookAttribution,
                calls: 1,
            }]
        );
        assert_eq!(
            (
                report.totals.selected_calls,
                report.totals.selected_unresolved_calls
            ),
            (3, 1)
        );
        // One ordered row per host identity; equal counts break on host, then
        // on the identity (kind first: builtin < hook < mcp).
        assert_eq!(
            report
                .identities
                .iter()
                .map(|r| (r.order, r.identity.name.as_str(), r.calls))
                .collect::<Vec<_>>(),
            [
                (0, "Read", 1),
                (1, HOOK_EVENT_NAME, 1),
                (2, "mcp__memhub__search", 1)
            ]
        );
        assert_eq!(
            report.identities[2].identity.server.as_deref(),
            Some("memhub")
        );
        // Home and the one local repository root; the remote and relative
        // stored values are refused, so only one repository root is read.
        assert_eq!(
            report
                .roots
                .iter()
                .map(|r| (r.scope, r.index, r.state))
                .collect::<Vec<_>>(),
            [
                (EnvRootScope::Home, 0, EnvRootState::Read),
                (EnvRootScope::Repository, 0, EnvRootState::Read),
            ]
        );
        assert_eq!(
            report
                .configured
                .iter()
                .map(|c| (c.host.as_str(), c.source, c.scope, c.name.as_str()))
                .collect::<Vec<_>>(),
            [
                (
                    "claude",
                    EnvConfigSource::ClaudeProjectMcpConfig,
                    EnvRootScope::Repository,
                    "beta"
                ),
                (
                    "cursor",
                    EnvConfigSource::CursorMcpConfig,
                    EnvRootScope::Home,
                    "alpha"
                ),
            ]
        );
        assert_eq!(report.totals.configured_components, 2);
        let text = serde_json::to_string(&report).unwrap();
        for private in [
            root.as_path().to_str().unwrap(),
            guard.path().to_str().unwrap(),
            "/secret/bin",
            "github.com",
            "xtrace.db",
        ] {
            assert!(!text.contains(private), "{private} was transported");
        }
    }
    assert!(windows.windows(2).all(|pair| pair[0] == pair[1]));

    // Enrichment and insertion are reread on the next call, never cached.
    writer
        .upsert_records("local", &[tool_call("c", &at(-600), "Read")], false)
        .unwrap();
    let refreshed = state.metrics_environment(7).unwrap();
    assert_eq!(refreshed.identities[0].identity.name, "Read");
    assert_eq!(refreshed.identities[0].calls, 2);
    assert_eq!(refreshed.totals.selected_calls, 4);

    assert!(matches!(
        state.metrics_environment(15),
        Err(StateError::InvalidMetricWindow)
    ));
    state.shutdown();
    assert!(matches!(
        state.metrics_environment(7),
        Err(StateError::Closed)
    ));
}

#[test]
fn environment_fixture_probe_rejects_empty_root_paths_and_preserves_relative_rules() {
    let guard = tempfile::TempDir::new().unwrap();
    let catalog = guard.path().canonicalize().unwrap();
    let base = catalog.join("F16/input/probes");
    std::fs::create_dir_all(base.join("env/home")).unwrap();
    std::fs::create_dir_all(base.join("env/repo")).unwrap();
    for (file, name) in [
        (".claude.json", "index-parent-home-server"),
        (".mcp.json", "index-parent-repo-server"),
        ("env/home/.claude.json", "nested-home-server"),
        ("env/repo/.mcp.json", "nested-repo-server"),
    ] {
        std::fs::write(
            base.join(file),
            serde_json::to_vec(&json!({"mcpServers":{name:{}}})).unwrap(),
        )
        .unwrap();
    }
    let write_index = |home: &str, repositories: &[&str]| {
        std::fs::write(
            base.join("env.json"),
            serde_json::to_vec(&json!({"roots":{"home":home,"repositories":repositories}}))
                .unwrap(),
        )
        .unwrap();
    };
    for (home, repositories, expected_names) in [
        (
            "env/home",
            vec!["env/repo"],
            vec!["nested-home-server", "nested-repo-server"],
        ),
        (
            "env/./home",
            vec!["env/./repo"],
            vec!["nested-home-server", "nested-repo-server"],
        ),
        ("env/home", vec![], vec!["nested-home-server"]),
    ] {
        write_index(home, &repositories);
        let probe = fixture_probe(&catalog).unwrap();
        assert_eq!(
            probe
                .components
                .iter()
                .map(|component| component.name.as_str())
                .collect::<Vec<_>>(),
            expected_names
        );
        assert_eq!(probe.roots.len(), repositories.len() + 1);
    }

    let absolute = base.join("env/home");
    let mut mismatches = Vec::new();
    for invalid in ["", absolute.to_str().unwrap(), "..", ".", "./env/home"] {
        for invalid_home in [true, false] {
            let home = if invalid_home { invalid } else { "env/home" };
            let repositories = if invalid_home {
                vec!["env/repo"]
            } else {
                vec![invalid]
            };
            write_index(home, &repositories);
            match fixture_probe(&catalog) {
                Err(StateError::FixtureInvalid) => {}
                Err(error) => panic!("unexpected fixture error: {error}"),
                Ok(probe) => {
                    let sentinel = if invalid_home {
                        "index-parent-home-server"
                    } else {
                        "index-parent-repo-server"
                    };
                    assert!(
                        probe
                            .components
                            .iter()
                            .any(|component| component.name == sentinel),
                        "accepted invalid root did not read its index-parent sentinel"
                    );
                    mismatches.push(format!(
                        "{} {invalid:?} succeeded and read {sentinel}",
                        if invalid_home { "home" } else { "repository" }
                    ));
                }
            }
        }
    }
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

#[cfg(all(debug_assertions, feature = "fixtures"))]
mod fixture {
    use super::*;
    use std::path::PathBuf;
    use xt_fixtures::Fixture;
    use xt_metrics::MetricsDb;
    use xtrace_desktop::environment::{assemble, fixture_probe};

    fn catalog() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../fixtures")
    }

    /// Metadata-only and full-content storage report identical environments.
    #[test]
    fn environment_metadata_only_and_content_databases_report_identically() {
        let f1 = Fixture::load(catalog().join("F1")).unwrap();
        let probe = fixture_probe(&catalog()).unwrap();
        let now = f1.now().timestamp_millis();
        let read = |keep_content: bool| {
            let db = f1.build_db(keep_content).unwrap();
            let metrics = MetricsDb::open(db.path()).unwrap();
            [7, 14, 30]
                .map(|days| {
                    assemble(
                        &metrics,
                        days,
                        now,
                        TimeZone::UTC,
                        MetricClock::Fixture,
                        &probe,
                    )
                    .unwrap()
                })
                .to_vec()
        };
        let metadata = read(false);
        assert_eq!(metadata, read(true));
        assert_eq!(metadata[0].totals.selected_calls, 5);
        assert_eq!(metadata[0].identities[0].identity.name, "Read");
    }

    /// Fixture mode reads F16's small synthetic matrix through the real probe,
    /// with the fixture's pinned clock, and transports no path.
    #[test]
    fn environment_fixture_state_uses_the_shared_probe_and_pinned_clock() {
        let root = tempfile::TempDir::new().unwrap();
        let state = AppState::build(
            StartupOptions {
                data_dir: Some(root.path().into()),
                fixture: Some("F1".into()),
                ..Default::default()
            },
            || panic!("fixture must not resolve live directory"),
            || panic!("fixture must not resolve the home directory"),
        )
        .unwrap();
        let now = Fixture::load(catalog().join("F1"))
            .unwrap()
            .now()
            .timestamp_millis();
        let probe = fixture_probe(&catalog()).unwrap();
        for days in [7, 14, 30] {
            let report = state.metrics_environment(days).unwrap();
            assert_eq!(report.window.end_ms, now);
            assert_eq!(report.window.clock, MetricClock::Fixture);
            assert_strip(&report);
            assert_eq!(report.strip_window.start_ms, now - 14 * DAY_MS);
            assert_eq!(
                serde_json::to_value(&report.configured).unwrap(),
                serde_json::to_value(&probe.components).unwrap()
            );
            assert_eq!(report.configured.len(), 14);
            let text = serde_json::to_string(&report).unwrap();
            assert!(!text.contains(&state.app_info().data_dir));
            assert!(!text.contains("fixtures") && !text.contains("SYNTHETIC-FIXTURE-VALUE"));
        }
    }
}
