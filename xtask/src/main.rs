use std::{
    cell::RefCell, env, error::Error, fs::OpenOptions, io::Write, path::PathBuf, process::ExitCode,
    sync::Arc,
};
use xt_fixtures::{Fixture, FixtureId, FixtureStatus};
use xtrace_desktop::{
    dto::{FixturePrEffortState, PrRef},
    pr_refresh::{PrRefreshService, RefreshOrigin, RefreshStorage},
    state::{StateError, pr_refresh_targets},
};

/// Every non-empty subset of this many listed pull requests is exported as an
/// M-19 state; a fixture that links more fails the export rather than leaving
/// the browser preview a selection it has no report for.
const MAX_STATE_PULL_REQUESTS: usize = 4;

/// The M-19 sections after the application's own fixture batch refreshes each
/// non-empty subset of `ids`, each from a freshly built, unrefreshed database.
/// Every attempt is stamped with the fixture's pinned instant and each pull
/// request's synthetic answer is fixed, so a sequence of partial refreshes ends
/// in the state of the union it refreshed.
fn pr_effort_states(
    fixture: &Fixture,
    listed: &[PrRef],
) -> Result<Vec<FixturePrEffortState>, Box<dyn Error>> {
    if listed.len() > MAX_STATE_PULL_REQUESTS {
        return Err(format!(
            "a shell fixture exports M-19 states for at most {MAX_STATE_PULL_REQUESTS} linked pull requests"
        )
        .into());
    }
    let now = fixture.now().timestamp_millis();
    let mut sorted = listed.to_vec();
    sorted.sort_unstable_by_key(|reference| reference.id);
    let mut states = Vec::new();
    for mask in 1_usize..(1 << sorted.len()) {
        let refreshed: Vec<PrRef> = sorted
            .iter()
            .enumerate()
            .filter(|(index, _)| mask & (1 << index) != 0)
            .map(|(_, reference)| reference.clone())
            .collect();
        let selection: Vec<i64> = refreshed.iter().map(|reference| reference.id).collect();
        let mut database = fixture.build_db(true)?;
        let snapshot = database.store().linked_pull_requests()?;
        {
            let store = RefCell::new(database.store_mut());
            PrRefreshService::fixture(now, Arc::new(|| {})).refresh_into(RefreshStorage {
                targets: &|| {
                    pr_refresh_targets(&snapshot, &selection).map_err(StateError::PrEncoding)
                },
                // Each state is what the user's own refresh from the dialog leaves.
                record: &|outcome| {
                    Ok(store
                        .borrow_mut()
                        .record_pr_refresh_from(outcome, RefreshOrigin::Manual)?)
                },
            })?;
        }
        states.push(FixturePrEffortState {
            sections: xtrace_desktop::dashboard::fixture_pr_effort(
                database.path(),
                now,
                fixture.snapshots().get("prices"),
            )?,
            analytics: xtrace_desktop::pr_analytics::fixture_pages(database.path(), now)?,
            refreshed,
        });
    }
    Ok(states)
}

const HELP: &str = "XTrace development tasks:
  cargo xtask fixture-validate [--catalog PATH]
  cargo xtask fixture-db F1 --out PATH [--catalog PATH]
  cargo xtask fixture-export ID [--shell] [--out PATH] [--catalog PATH]
  cargo xtask dto-export [--out DIRECTORY]

Validation reports skeletons as unimplemented, not accepted rules.
Database output requires a populated fixture and never overwrites files.
Canonical exports remain available; --shell exports the shared app IPC structs.";

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::from(2)
        }
    }
}

fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let Some(task) = args.first().map(String::as_str) else {
        println!("{HELP}");
        return Ok(());
    };
    if matches!(task, "help" | "--help" | "-h") && args.len() == 1 {
        println!("{HELP}");
        return Ok(());
    }
    if task == "dto-export" {
        let directory = match &args[1..] {
            [] => PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../apps/desktop/ui/src/data/generated"),
            [option, directory] if option == "--out" => PathBuf::from(directory),
            _ => return Err("dto-export accepts only --out DIRECTORY".into()),
        };
        xtrace_desktop::dto::export_types(directory)?;
        return Ok(());
    }
    if !matches!(task, "fixture-validate" | "fixture-db" | "fixture-export") {
        return Err(format!("unknown task {task}; run cargo xtask help").into());
    }
    let mut rest = args[1..].iter();
    let id = if task != "fixture-validate" {
        Some(FixtureId::parse(
            rest.next().ok_or("fixture ID is required")?,
        )?)
    } else {
        None
    };
    let mut catalog = None;
    let mut output = None;
    let mut shell = false;
    while let Some(option) = rest.next() {
        if option == "--shell" && task == "fixture-export" && !shell {
            shell = true;
            continue;
        }
        let target = match option.as_str() {
            "--catalog" => &mut catalog,
            "--out" if task != "fixture-validate" => &mut output,
            _ => return Err(format!("unexpected option {option}").into()),
        };
        if target.is_some() {
            return Err(format!("duplicate option {option}").into());
        }
        let value = rest
            .next()
            .filter(|value| !value.starts_with("--"))
            .ok_or_else(|| format!("missing value for {option}"))?;
        *target = Some(PathBuf::from(value));
    }
    let catalog =
        catalog.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures"));
    if task == "fixture-validate" {
        let fixtures = Fixture::all(catalog)?;
        let mut asserted = 0;
        for fixture in &fixtures {
            let id = fixture.manifest().id;
            match fixture.manifest().status {
                FixtureStatus::Skeleton => {
                    println!("{id}: SKELETON — structure valid; acceptance UNIMPLEMENTED")
                }
                FixtureStatus::Populated => {
                    fixture.assert_reference()?;
                    asserted += 1;
                    println!(
                        "{id}: POPULATED — reference assertions passed ({} golden keys)",
                        fixture.expected().len()
                    );
                }
            }
        }
        println!(
            "{} entries structurally valid; {asserted} populated/asserted; {} skeleton/unimplemented. Product metric acceptance is not implied.",
            fixtures.len(),
            fixtures.len() - asserted
        );
        return Ok(());
    }
    let id = id.unwrap();
    let fixture = Fixture::load(catalog.join(id.to_string()))?;
    if fixture.manifest().id != id {
        return Err("manifest ID does not match requested fixture".into());
    }
    if task == "fixture-db" {
        let path = output.ok_or("fixture-db requires --out PATH")?;
        fixture.write_db(&path, true)?;
        println!(
            "{id}: wrote file-backed fixture database to {}",
            path.display()
        );
    } else {
        let json = if shell {
            let mut database = fixture.build_db(true)?;
            // Dashboards are read before the refresh, as a fixture database
            // starts: a refresh is only ever an explicit request, so nothing
            // shows its facts until one is made.
            let dashboards = xtrace_desktop::dashboard::fixture_reports(
                database.path(),
                fixture.now().timestamp_millis(),
                fixture.snapshots().get("prices"),
            )?;
            // Sessions, like the Dashboards, are read before the refresh: a
            // listed session's pull-request titles are refresh-owned, so a
            // fixture database shows none until a refresh is requested.
            let sessions = xtrace_desktop::dto::fixture_session_pages(
                database.path(),
                fixture.now().timestamp_millis(),
                fixture.snapshots().get("prices"),
            )?;
            // The PRs page report before the refresh, like the Dashboards.
            let pr_analytics = xtrace_desktop::pr_analytics::fixture_pages(
                database.path(),
                fixture.now().timestamp_millis(),
            )?;
            let session_stretches = xtrace_desktop::dto::fixture_session_stretches(
                database.path(),
                fixture.now().timestamp_millis(),
                fixture.snapshots().get("prices"),
            )?;
            // Span details are metadata of the stored records, which a
            // pull-request refresh never changes.
            let span_details = xtrace_desktop::dashboard::fixture_span_details(
                database.path(),
                fixture.now().timestamp_millis(),
                fixture.snapshots().get("prices"),
            )?;
            // Drilldowns are read before the refresh too, like the Sessions
            // pages: membership is the links, which a refresh never changes,
            // and the preview takes each link's refresh-owned title from its
            // current pull-request state, as the Sessions list does.
            let listed = xtrace_desktop::state::pr_list(database.store().linked_pull_requests()?)?;
            let pr_sessions = xtrace_desktop::pr_analytics::fixture_sessions(
                database.path(),
                fixture.now().timestamp_millis(),
                &listed
                    .rows
                    .iter()
                    .map(|row| {
                        u64::try_from(row.pull_request.number)
                            .map(|number| (row.pull_request.repository.clone(), number))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                fixture.snapshots().get("prices"),
            )?;
            // The listed rows are what the fixture stores before the refresh,
            // and the refreshed rows are what it leaves.
            let pull_requests = xtrace_desktop::pr_refresh::fixture_reports(
                database.store_mut(),
                fixture.now().timestamp_millis(),
            )?;
            let pr_effort_states = pr_effort_states(
                &fixture,
                &pull_requests
                    .listed
                    .rows
                    .iter()
                    .map(|row| row.pull_request.clone())
                    .collect::<Vec<_>>(),
            )?;
            let export = xtrace_desktop::dto::FixtureExport {
                app_info: xtrace_desktop::dto::AppInfo {
                    name: "XTrace Desktop".into(),
                    version: env!("CARGO_PKG_VERSION").into(),
                    data_dir: format!("fixture://{id}"),
                    fixture: Some(id.to_string()),
                    schema_version: database.store().schema_version()?,
                    listening: false,
                    had_indexed_history_at_startup: database.store().counts()?.sessions > 0,
                },
                dashboards,
                environments: xtrace_desktop::environment::fixture_reports(
                    database.path(),
                    fixture.now().timestamp_millis(),
                    &catalog,
                )?,
                today: xtrace_desktop::today::fixture_today(
                    database.path(),
                    fixture.now().timestamp_millis(),
                    fixture.snapshots().get("prices"),
                )?,
                db_counts: database.store().counts()?.try_into()?,
                sessions_summaries: xtrace_desktop::dto::fixture_sessions_summaries(
                    database.path(),
                    fixture.now().timestamp_millis(),
                )?,
                sessions,
                session_stretches,
                span_details,
                native_index: xtrace_desktop::dto::NativeIndexStatus {
                    phase: xtrace_desktop::dto::NativeIndexPhase::Disabled {
                        reason: "fixture mode uses a disposable database".into(),
                    },
                    freshness: xtrace_desktop::dto::NativeFreshness::Unknown,
                    python: xtrace_desktop::dto::PythonRuntime::Missing {
                        reason: "not resolved: the index is disabled".into(),
                    },
                    readers: xtrace_desktop::dto::ReaderBundle::Unavailable {
                        reason: "not verified: the index is disabled".into(),
                    },
                    hosts: Vec::new(),
                    needs_attention: false,
                    reconciles: 0,
                    files_scanned: 0,
                },
                pull_requests: pull_requests.listed,
                pr_refresh: pull_requests.refresh,
                pull_requests_refreshed: pull_requests.refreshed,
                pr_effort_states,
                pr_analytics,
                pr_sessions,
                // Fixture startup selects no native home, so its rule activity
                // service has no source to read.
                rule_activity: xtrace_desktop::rule_activity::RuleActivityService::new(None)
                    .read("fixture"),
            };
            serde_json::to_string_pretty(&export)?
        } else {
            serde_json::to_string_pretty(&fixture.export())?
        } + "\n";
        if let Some(path) = output {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)?;
            file.write_all(json.as_bytes())?;
            println!("{id}: wrote fixture export to {}", path.display());
        } else {
            print!("{json}");
        }
    }
    Ok(())
}
