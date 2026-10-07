#[cfg(all(debug_assertions, feature = "fixtures"))]
use xtrace_desktop::dto::{FixtureExport, FixtureSessionStretches};
use xtrace_desktop::state::{AppState, StartupOptions, StateError};

/// Fixture startup never indexes live data: the shell export carries the
/// disabled index the app reports in fixture mode.
#[cfg(all(debug_assertions, feature = "fixtures"))]
fn expected_native_index() -> xtrace_desktop::dto::NativeIndexStatus {
    use std::sync::Arc;
    use xtrace_desktop::native_index::NativeIndex;
    NativeIndex::disabled("fixture mode uses a disposable database", Arc::new(|_| {})).status()
}

#[test]
fn fixture_mode_data_dir_override_wins_and_live_store_persists() {
    let root = tempfile::TempDir::new().unwrap();
    let path = root.path().join("explicit");
    std::fs::create_dir_all(root.path().join("home")).unwrap();
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(path.clone()),
            ..Default::default()
        },
        || panic!("default path must not be resolved"),
        || Ok(root.path().join("home")),
    )
    .unwrap();
    assert_eq!(state.app_info().data_dir, path.to_str().unwrap());
    assert_eq!(state.app_info().schema_version, 23);
    assert_eq!(state.app_info().fixture, None);
    assert!(!state.app_info().listening);
    assert!(!state.app_info().had_indexed_history_at_startup);
    assert_eq!(
        serde_json::to_value(state.db_counts().unwrap()).unwrap(),
        serde_json::json!({"sessions":0,"records":0,"usage":0})
    );
    state.shutdown();
    state.shutdown();
    assert!(matches!(state.db_counts(), Err(StateError::Closed)));
    assert!(path.join("xtrace.db").exists());
    let home = root.path().join("home");
    AppState::build(StartupOptions::default(), || Ok(path), || Ok(home)).unwrap();
}

#[test]
fn fixture_mode_options_have_explicit_precedence_and_validation() {
    let options =
        StartupOptions::parse(None, Some("F2".into()), ["--fixture".into(), "F1".into()]).unwrap();
    assert_eq!(options.fixture.as_deref(), Some("F1"));
    for args in [
        vec!["--fixture"],
        vec!["--fixture="],
        vec!["--fixture", "--fixture"],
        vec!["--fixture=F1", "--fixture=F2"],
    ] {
        assert!(StartupOptions::parse(None, None, args.into_iter().map(String::from)).is_err());
    }
    assert!(StartupOptions::parse(None, Some(String::new().into()), []).is_err());
}

#[cfg(not(all(debug_assertions, feature = "fixtures")))]
#[test]
fn fixture_mode_is_disabled_without_debug_feature() {
    let result = AppState::build(
        StartupOptions {
            fixture: Some("F1".into()),
            ..Default::default()
        },
        || panic!("fixture must not resolve live directory"),
        || panic!("fixture must not resolve the home directory"),
    );
    assert!(matches!(result, Err(StateError::FixtureDisabled)));
}

/// Every listed session's links carry exactly the title the PR list holds for
/// the same canonical identity in the same state, in every window.
#[cfg(all(debug_assertions, feature = "fixtures"))]
fn assert_links_follow(state: &AppState, list: &xtrace_desktop::dto::PrList) {
    let titles: std::collections::BTreeMap<_, _> = list
        .rows
        .iter()
        .map(|row| {
            (
                (row.pull_request.repository.clone(), row.pull_request.number),
                row.title.clone(),
            )
        })
        .collect();
    let mut seen = 0;
    for days in [7, 14, 30] {
        for row in state.sessions_list("", None, None, days).unwrap().rows {
            for link in row.pr_links {
                let key = (link.repository.clone(), i64::try_from(link.number).unwrap());
                assert_eq!(Some(&link.title), titles.get(&key), "{key:?} over {days}d");
                seen += 1;
            }
        }
    }
    assert!(seen > 0);
}

#[cfg(all(debug_assertions, feature = "fixtures"))]
#[test]
fn fixture_mode_builds_isolated_database_and_generated_export_parity() {
    let root = tempfile::TempDir::new().unwrap();
    let build = || {
        AppState::build(
            StartupOptions {
                data_dir: Some(root.path().into()),
                fixture: Some("F1".into()),
                ..Default::default()
            },
            || panic!("fixture must not resolve live directory"),
            || panic!("fixture must not resolve the home directory"),
        )
        .unwrap()
    };
    let first = build();
    let second = build();
    assert_ne!(first.app_info().data_dir, second.app_info().data_dir);
    let path = std::path::PathBuf::from(first.app_info().data_dir.clone());
    let mut app_info = first.app_info();
    assert_eq!(app_info.fixture.as_deref(), Some("F1"));
    assert_eq!(app_info.version, env!("CARGO_PKG_VERSION"));
    // Keep the checked-in fixture's data unchanged across app releases.
    app_info.version =
        serde_json::from_str::<serde_json::Value>(include_str!("../../ui/fixtures/F1.json"))
            .unwrap()["app_info"]["version"]
            .as_str()
            .unwrap()
            .to_owned();
    app_info.data_dir = "fixture://F1".into();
    // The exported pull-request reports are what this native fixture state
    // produces through the ordinary refresh command, with the fixture's own
    // pinned instant: no GitHub CLI is resolved, nothing is launched.
    // Dashboards before any refresh: a fixture database starts with links
    // only, so M-19 has no cached facts yet and says so.
    let dashboards: Vec<_> = [7, 14, 30]
        .into_iter()
        .map(|days| first.metrics_dashboard(days).unwrap())
        .collect();
    // Sessions before any refresh too: a link's title is refresh-owned, so a
    // fixture database lists every link without one until a refresh is asked.
    // The same command the frontend calls, once per window preset: the
    // exported pages must equal what a running app answers.
    let sessions: Vec<_> = [7, 14, 30]
        .into_iter()
        .map(|days| first.sessions_list("", None, None, days).unwrap())
        .collect();
    // The stretches command, for every session each window lists: a
    // fixture's timeline must be what a running app would answer for the
    // same session and the same window.
    let session_stretches: Vec<_> = [7, 14, 30]
        .into_iter()
        .flat_map(|days| {
            first
                .sessions_list("", None, None, days)
                .unwrap()
                .rows
                .into_iter()
                .map(move |row| (days, row.id))
        })
        .map(|(days, session_id)| FixtureSessionStretches {
            window_days: days,
            stretches: first.session_stretches(&session_id, days).unwrap(),
            session_id,
        })
        .collect();

    // The span detail command, for every span the lanes return: the lane axis
    // is fixed, so every preset returns the same spans.
    let span_details: Vec<_> = dashboards[0]
        .lanes
        .iter()
        .map(|lane| xtrace_desktop::dto::FixtureSpanDetail {
            session_id: lane.session_id.clone(),
            start_ms: lane.start_ms,
            end_ms: lane.end_ms,
            detail: first
                .span_detail(
                    &lane.session_id,
                    lane.start_ms,
                    lane.end_ms,
                    &xtrace_desktop::transcript_reads::TranscriptReads::default(),
                    "span",
                    &xtrace_desktop::native_index::DetailReaders::disabled(),
                )
                .unwrap(),
        })
        .collect();
    assert!(!span_details.is_empty(), "F1 draws at least one lane span");
    // A request no lane could have sent is refused before storage is read.
    let lane = &dashboards[0].lanes[0];
    for (session_id, start_ms, end_ms) in [
        ("", lane.start_ms, lane.end_ms),
        (lane.session_id.as_str(), lane.end_ms + 1, lane.end_ms),
        (
            lane.session_id.as_str(),
            lane.end_ms - xtrace_desktop::dashboard::LANE_WINDOW_MS - 1,
            lane.end_ms,
        ),
    ] {
        assert!(matches!(
            first.span_detail(
                session_id,
                start_ms,
                end_ms,
                &xtrace_desktop::transcript_reads::TranscriptReads::default(),
                "span",
                &xtrace_desktop::native_index::DetailReaders::disabled(),
            ),
            Err(StateError::InvalidSpanRequest)
        ));
    }

    // The PRs page report, per range and confidence mode, before any refresh:
    // the same command the page calls.
    let analytics = |state: &AppState| -> Vec<_> {
        [7, 14, 30]
            .into_iter()
            .flat_map(|days| [false, true].map(|confirmed| (days, confirmed)))
            .map(|(days, confirmed)| state.pr_analytics(days, confirmed).unwrap())
            .collect()
    };
    let pr_analytics = analytics(&first);
    let pull_requests = first.pr_list().unwrap();
    // Each listed pull request's drilldown, pinned to the end of the report
    // the page would have shown for the same range and confidence mode.
    let pr_sessions: Vec<_> = pull_requests
        .rows
        .iter()
        .flat_map(|row| {
            [7, 14, 30]
                .into_iter()
                .flat_map(|days| [false, true].map(|confirmed| (days, confirmed)))
                .map(move |(days, confirmed)| (row.pull_request.clone(), days, confirmed))
        })
        .map(|(reference, days, confirmed)| {
            let anchor = pr_analytics
                .iter()
                .find(|page| page.window.days == days && page.report.confirmed_only == confirmed)
                .unwrap()
                .window
                .end_ms;
            let number = u64::try_from(reference.number).unwrap();
            xtrace_desktop::dto::FixturePrSessions {
                page: first
                    .pr_sessions(xtrace_desktop::pr_analytics::PrSessionsRequest {
                        repository: &reference.repository,
                        number,
                        confirmed_only: confirmed,
                        window_days: days,
                        window_end_ms: anchor,
                        after: None,
                    })
                    .unwrap(),
                repository: reference.repository,
                number,
                confirmed_only: confirmed,
                window_days: days,
                window_end_ms: anchor,
            }
        })
        .collect();
    let selection: Vec<i64> = pull_requests
        .rows
        .iter()
        .map(|row| row.pull_request.id)
        .collect();
    let pr_refresh = xtrace_desktop::pr_refresh::PrRefreshService::fixture(
        first
            .fixture_now_ms()
            .expect("fixture mode pins an instant"),
        std::sync::Arc::new(|| {}),
    )
    .refresh(&first, &selection)
    .unwrap();
    let pull_requests_refreshed = first.pr_list().unwrap();
    assert_links_follow(&first, &pull_requests_refreshed);
    // Every non-empty subset of the listed pull requests, each refreshed by
    // the ordinary command on its own freshly built fixture state: the
    // exported M-19 states must equal what a running app answers after it.
    let now_ms = first
        .fixture_now_ms()
        .expect("fixture mode pins an instant");
    let mut sorted: Vec<_> = pull_requests
        .rows
        .iter()
        .map(|row| row.pull_request.clone())
        .collect();
    sorted.sort_unstable_by_key(|reference| reference.id);
    let pr_effort_states = (1_usize..(1 << sorted.len()))
        .map(|mask| {
            let refreshed: Vec<_> = sorted
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, reference)| reference.clone())
                .collect();
            let subset: Vec<i64> = refreshed.iter().map(|reference| reference.id).collect();
            let state = build();
            xtrace_desktop::pr_refresh::PrRefreshService::fixture(
                now_ms,
                std::sync::Arc::new(|| {}),
            )
            .refresh(&state, &subset)
            .unwrap();
            // One selected, a failed one, any mix, all: the list's titles are
            // the PR list's own for this state.
            assert_links_follow(&state, &state.pr_list().unwrap());
            xtrace_desktop::dto::FixturePrEffortState {
                analytics: analytics(&state),
                sections: [7, 14, 30]
                    .into_iter()
                    .map(|days| {
                        let report = state.metrics_dashboard(days).unwrap();
                        xtrace_desktop::dto::FixtureRefreshedPrEffort {
                            days,
                            merged_prs: report.tiles.merged_prs,
                            pr_effort: report.pr_effort,
                        }
                    })
                    .collect(),
                refreshed,
            }
        })
        .collect();
    let export = FixtureExport {
        app_info,
        db_counts: first.db_counts().unwrap(),
        native_index: expected_native_index(),
        sessions,
        session_stretches,
        span_details,
        dashboards,
        environments: [7, 14, 30]
            .into_iter()
            .map(|days| first.metrics_environment(days).unwrap())
            .collect(),
        pull_requests,
        pr_refresh,
        pull_requests_refreshed,
        pr_effort_states,
        pr_analytics,
        pr_sessions,
        today: first.today().unwrap(),
        // Built as fixture startup builds it: from the state's native home,
        // which fixture mode never has.
        rule_activity: {
            assert_eq!(first.native_home(), None);
            xtrace_desktop::rule_activity::RuleActivityService::new(first.native_home())
                .read("fixture")
        },
    };
    // Compare the exact generated bytes: serde_json's default float parser is
    // not round-trip exact, so parsed values can differ in the last bit.
    assert_eq!(
        serde_json::to_string_pretty(&export).unwrap() + "\n",
        include_str!("../../ui/fixtures/F1.json")
    );
    let exported = serde_json::to_value(export).unwrap();
    assert_eq!(
        exported["db_counts"],
        serde_json::json!({"sessions":1,"records":25,"usage":15})
    );
    // F1's one session, measured over its pinned seven-day window.
    assert_eq!(
        exported["sessions"][0]["rows"][0]["metrics"],
        serde_json::json!({
            "state": "indexed",
            "events": 25,
            "human_messages": 5,
            "tool_calls": 5,
            "agent_ms": 1_380_000,
            "tokens": {
                "selected_responses": 10, "measured_responses": 10,
                "sessions": 1, "measured_sessions": 1,
                "counters": {"input_tokens": 750, "output_tokens": 150, "cache_read_tokens": 150,
                             "cache_creation_tokens": 50, "total_tokens": 1100}
            }
        })
    );
    // The design columns beside them: the native start the host recorded, no
    // invented title, every stored link with its own confidence, and the
    // median of the same stretches the detail export carries.
    let row = &exported["sessions"][0]["rows"][0];
    assert_eq!(row["title"], serde_json::Value::Null);
    assert!(row["started_at_ms"].is_i64());
    assert_eq!(row["pr_links"].as_array().unwrap().len(), 3);
    assert!(
        row["pr_links"]
            .as_array()
            .unwrap()
            .iter()
            .all(|link| link["confidence"].is_string() && link["url"].is_string())
    );
    let stretches = exported["session_stretches"][0]["stretches"]["stretches"]
        .as_array()
        .unwrap();
    assert_eq!(row["hands_off"]["n"], stretches.len());
    let mut durations: Vec<u64> = stretches
        .iter()
        .map(|s| s["duration_ms"].as_u64().unwrap())
        .collect();
    durations.sort_unstable();
    assert_eq!(
        row["hands_off"]["median_min"].as_f64().unwrap(),
        durations[durations.len() / 2] as f64 / 60000.0
    );
    // The listed row's numbers are the Dashboard's, restricted to that session.
    let dashboard = &exported["dashboards"][0]["tiles"];
    assert_eq!(dashboard["human_messages"]["value"], 5.0);
    assert_eq!(dashboard["tokens"]["value"], 1100.0);
    // The exported daily hours are the hero's own totals, split across the days
    // the export reports, so a UI reading the fixture reads the same numbers a
    // running app does. F1 records all of its work on one local date.
    for report in exported["dashboards"].as_array().unwrap() {
        let days = report["days"].as_array().unwrap();
        let agent: f64 = days
            .iter()
            .map(|day| day["agent_hours"].as_f64().unwrap())
            .sum();
        let human: f64 = days
            .iter()
            .map(|day| day["human_hours_est"].as_f64().unwrap())
            .sum();
        assert!((agent - report["tiles"]["agent_hours"]["value"].as_f64().unwrap()).abs() < 1e-9);
        assert!(
            (human
                - report["tiles"]["human_hours_est"]["value"]
                    .as_f64()
                    .unwrap())
            .abs()
                < 1e-9
        );
        let worked: Vec<_> = days
            .iter()
            .filter(|day| day["agent_hours"] != 0.0)
            .collect();
        assert_eq!(worked.len(), 1);
        assert_eq!(worked[0]["date"], "2026-09-07");
        assert_eq!(worked[0]["agent_hours"], 1_380_000.0 / 3_600_000.0);
    }
    assert_eq!(
        exported["sessions"][0]["window"],
        exported["dashboards"][0]["window"]
    );
    assert!(first.sessions_list("", None, None, 15).is_err());
    first.shutdown();
    assert!(!path.exists());
    assert!(matches!(first.db_counts(), Err(StateError::Closed)));
    first.shutdown();
    assert!(std::path::Path::new(&second.app_info().data_dir).exists());
    let second_path = std::path::PathBuf::from(second.app_info().data_dir);
    drop(second);
    assert!(!second_path.exists());
}

#[cfg(all(debug_assertions, feature = "fixtures"))]
#[test]
fn fixture_mode_rejects_skeletons_and_invalid_ids_without_live_reads() {
    for fixture in ["F2", "F20", "../F1", "F999"] {
        let result = AppState::build(
            StartupOptions {
                fixture: Some(fixture.into()),
                ..Default::default()
            },
            || panic!("fixture must not resolve live directory"),
            || panic!("fixture must not resolve the home directory"),
        );
        assert!(matches!(result, Err(StateError::FixtureInvalid)));
    }
}

#[cfg(unix)]
#[test]
fn fixture_mode_non_unicode_selection_fails_before_live_startup() {
    use std::os::unix::ffi::OsStringExt;
    let value = std::ffi::OsString::from_vec(vec![0xff]);
    assert!(matches!(
        StartupOptions::parse(None, Some(value), []),
        Err(StateError::InvalidOption)
    ));
}

#[test]
fn a_data_directory_inside_the_native_history_is_refused_before_anything_is_written() {
    // The database destination is validated against the native home before
    // the directory is created or SQLite opens: nothing lands in `.claude`.
    let root = tempfile::TempDir::new().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(home.join(".claude/projects")).unwrap();
    let inside = home.join(".claude/xtrace-data");
    let result = AppState::build(
        StartupOptions {
            data_dir: Some(inside.clone()),
            native_home: Some(home.clone()),
            ..Default::default()
        },
        || panic!("an explicit data directory needs no default"),
        || panic!("an explicit home needs no default"),
    );
    assert!(
        matches!(result, Err(StateError::IndexDestination(reason)) if reason.contains("native history")),
        "{:?}",
        result.err()
    );
    assert!(
        !inside.exists(),
        "no directory was created inside the native history"
    );
    assert!(
        std::fs::read_dir(home.join(".claude"))
            .unwrap()
            .all(|entry| entry.unwrap().file_name() == "projects"),
        "no database files were written into the native history"
    );
    // The same home with a data directory beside it opens normally.
    let beside = root.path().join("data");
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(beside.clone()),
            native_home: Some(home.clone()),
            ..Default::default()
        },
        || panic!("an explicit data directory needs no default"),
        || panic!("an explicit home needs no default"),
    )
    .unwrap();
    assert_eq!(state.native_home(), Some(home.as_path()));
    assert_eq!(
        state.database_path(),
        Some(beside.join("xtrace.db").as_path())
    );
    state.shutdown();
}

#[cfg(unix)]
#[test]
fn an_unreadable_project_directory_does_not_prevent_startup() {
    use std::os::unix::fs::PermissionsExt;
    // A source-access problem is the index's to report, not a startup failure.
    let root = tempfile::TempDir::new().unwrap();
    let home = root.path().join("home");
    let sealed = home.join(".claude/projects/-repo-sealed");
    std::fs::create_dir_all(&sealed).unwrap();
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o000)).unwrap();
    let built = AppState::build(
        StartupOptions {
            data_dir: Some(root.path().join("data")),
            native_home: Some(home.clone()),
            ..Default::default()
        },
        || panic!("an explicit data directory needs no default"),
        || panic!("an explicit home needs no default"),
    );
    std::fs::set_permissions(&sealed, std::fs::Permissions::from_mode(0o755)).unwrap();
    let state = built.unwrap();
    assert_eq!(state.native_home(), Some(home.as_path()));
    state.shutdown();
}

#[test]
fn a_native_home_that_does_not_exist_is_refused_at_startup() {
    // An absent home could not be watched (there is no ancestor to cover
    // it), so the index would stay degraded and empty; refuse it instead.
    let root = tempfile::TempDir::new().unwrap();
    let result = AppState::build(
        StartupOptions {
            data_dir: Some(root.path().join("data")),
            native_home: Some(root.path().join("absent-home")),
            ..Default::default()
        },
        || panic!("an explicit data directory needs no default"),
        || panic!("an explicit home needs no default"),
    );
    assert!(
        matches!(result, Err(StateError::NativeHome)),
        "{:?}",
        result.err()
    );
    assert!(!root.path().join("data").exists());
}

#[cfg(unix)]
#[test]
fn a_dangling_source_root_alias_does_not_prevent_startup() {
    // `~/.codex` pointing nowhere is the Codex scan's to report as missing.
    let root = tempfile::TempDir::new().unwrap();
    let home = root.path().join("home");
    std::fs::create_dir_all(home.join(".claude/projects")).unwrap();
    std::os::unix::fs::symlink(root.path().join("absent"), home.join(".codex")).unwrap();
    let state = AppState::build(
        StartupOptions {
            data_dir: Some(root.path().join("data")),
            native_home: Some(home.clone()),
            ..Default::default()
        },
        || panic!("an explicit data directory needs no default"),
        || panic!("an explicit home needs no default"),
    )
    .unwrap();
    assert_eq!(state.native_home(), Some(home.as_path()));
    state.shutdown();
}
