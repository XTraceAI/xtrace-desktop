//! Offline acceptance of the display check on a disposable copy: the native
//! index's own import and background passes, and the app's own public
//! Sessions and Dashboard responses after each of them.
//!
//! ```text
//! cargo run -p xtrace-desktop --example visibility_snapshots -- \
//!     --data-dir DIRECTORY_HOLDING_A_COPIED_xtrace.db --home COPIED_HOME \
//!     --out FRAMES.json [--at UTC_MS | --live-clock] [--days 7] \
//!     [--watch SESSION_ID]... [--legacy IDS_FILE] [--restart] \
//!     [--bundle AGENT_PLUGINS] [--max-passes N]
//! ```
//!
//! The readers are the installed app's bundled producer
//! (`/Applications/XTrace Desktop.app/Contents/Resources/agent-plugins`
//! unless `--bundle` names another), verified against the pin compiled into
//! the app ([`xtrace_desktop::native_index::PIN`]) before anything is opened.
//! Stages, each followed by a frame: the copy opened (its migration applied),
//! the ordinary scan of every host, then every bounded background pass until
//! the work is quiet; with `--restart`, a restarted worker's scan and passes
//! again. A frame holds the public responses read then: the Dashboard report,
//! and for each `--watch` session the Sessions page searched for its exact
//! identity, as the app's commands answer them. With `--at` they are the
//! production response builders the commands call, at that fixed instant in
//! UTC; with `--live-clock`, the app state's own commands at the system clock.
//! Every frame also holds a digest of the whole Sessions list's membership,
//! cursors and measurements — every field of every page but the display
//! fields this work adds — and, for `--legacy` identities (one per line),
//! each one's display state.
//!
//! Only identities, hosts, states, counts, times and measurements are
//! written: every title, repository, branch, path, URL and text is blanked.
//! The index must be a copy outside the app's own data folder, and the home
//! must not be the real home; sources are only read.
use std::{
    collections::BTreeMap,
    env,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::Instant,
};
use xt_ingest::native::{
    ImportRequest, ProducerSource, ScanMode,
    readers_cli::{parse_pin, verify_bundle},
    scan_native_continued,
    session_creation::{SpawnBacklog, SpawnProgress, continue_codex_spawns, spawn_limits},
    validate_index_destination,
};
use xt_store::{Host, Store};
use xtrace_desktop::{
    dto::{MetricClock, SessionPage, SessionQuery},
    state::{AppState, StartupOptions},
};

const INSTALLED_BUNDLE: &str = "/Applications/XTrace Desktop.app/Contents/Resources/agent-plugins";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let all = |flag: &str| -> Vec<String> {
        args.windows(2)
            .filter(|pair| pair[0] == flag)
            .map(|pair| pair[1].clone())
            .collect()
    };
    let usage = "usage: visibility_snapshots --data-dir DIRECTORY --home HOME --out FRAMES.json \
                 [--at UTC_MS | --live-clock] [--days N] [--watch ID]... [--legacy FILE] \
                 [--restart] [--bundle AGENT_PLUGINS] [--max-passes N]";
    let (Some(data_dir), Some(home), Some(out)) =
        (value("--data-dir"), value("--home"), value("--out"))
    else {
        return Err(usage.into());
    };
    let clock = match (value("--at"), args.iter().any(|arg| arg == "--live-clock")) {
        (Some(at), false) => Clock::Fixed(at.parse()?),
        (None, true) => Clock::Live,
        _ => return Err(usage.into()),
    };
    let days: u32 = value("--days")
        .map(|days| days.parse())
        .transpose()?
        .unwrap_or(7);
    let max_passes: usize = value("--max-passes")
        .map(|passes| passes.parse())
        .transpose()?
        .unwrap_or(2000);
    let watch = all("--watch");
    let legacy: Vec<String> = match value("--legacy") {
        Some(file) => std::fs::read_to_string(file)?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect(),
        None => Vec::new(),
    };
    let home = resolved(Path::new(&home))?;
    let data_dir = resolved(Path::new(&data_dir))?;
    let db = data_dir.join("xtrace.db");
    guard(&db, &home)?;
    // The installed app's readers, checked against the app's own pin.
    let bundle = PathBuf::from(value("--bundle").unwrap_or_else(|| INSTALLED_BUNDLE.into()));
    let pin = parse_pin(xtrace_desktop::native_index::PIN)?;
    verify_bundle(&pin, &bundle)?;
    let producer = ProducerSource::Bundle { pin, root: bundle };

    let started = Instant::now();
    let mut store = Store::open(&db)?;
    let state = AppState::build(
        StartupOptions::default().with_native_environment(Some(home.clone().into()), None)?,
        || Ok(data_dir.clone()),
        || Ok(home.clone()),
    )?;
    let reader = Reader {
        db: &db,
        state: &state,
        clock,
        days,
        watch: &watch,
        legacy: &legacy,
    };
    let mut frames = vec![reader.frame("opened", None)?];
    let hosts = [Host::Claude, Host::Codex, Host::Cursor];
    let mut stages = vec![false];
    if args.iter().any(|arg| arg == "--restart") {
        stages.push(true);
    }
    let mut quiet = Vec::new();
    for restart in stages {
        let label = if restart { "restart " } else { "" };
        let mut spawns = SpawnBacklog::starting();
        let report = scan_native_continued(
            &mut store,
            &ImportRequest {
                home: &home,
                hosts: &hosts,
                producer: &producer,
                python: None,
                observed_at: now_ms(),
                cancel: None,
            },
            ScanMode::Resume,
            &mut |_| {},
            &mut spawns,
            spawn_limits(),
        );
        let hosts: Vec<serde_json::Value> = report
            .hosts
            .iter()
            .map(|host| {
                serde_json::json!({"host": host.host, "status": host.status,
                    "sessions": host.sessions.len()})
            })
            .collect();
        frames.push(reader.frame(&format!("{label}indexed"), Some(serde_json::json!(hosts)))?);
        let mut passes = 0;
        while spawns.pending() && passes < max_passes {
            let progress = continue_codex_spawns(
                &mut store,
                &home,
                &mut spawns,
                spawn_limits(),
                None,
                now_ms(),
            )?;
            passes += 1;
            frames.push(reader.frame(&format!("{label}pass {passes}"), Some(counts(&progress)))?);
        }
        quiet.push(!spawns.pending());
    }
    // Consecutive identical frames are kept once, with how many passes they
    // stood for.
    let kept = coalesce(frames);
    std::fs::write(
        &out,
        serde_json::to_vec(&serde_json::json!({
            "clock": match clock { Clock::Fixed(at) => serde_json::json!(at), Clock::Live => serde_json::json!("live") },
            "days": days,
            "watch": watch,
            "quiet": quiet,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "frames": kept,
        }))?,
    )?;
    println!(
        "{}",
        serde_json::json!({"frames": kept_len(&out)?, "quiet": quiet,
            "elapsed_ms": started.elapsed().as_millis() as u64})
    );
    Ok(())
}

fn coalesce(frames: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
    let mut kept: Vec<serde_json::Value> = Vec::new();
    for frame in frames {
        match kept.last_mut() {
            Some(last)
                if last["responses"] == frame["responses"]
                    && last["legacy"] == frame["legacy"]
                    && last["progress"] == frame["progress"] =>
            {
                let repeats = last["repeats"].as_u64().unwrap_or(0) + 1;
                last["repeats"] = serde_json::json!(repeats);
                last["through"] = frame["at"].clone();
            }
            _ => kept.push(frame),
        }
    }
    kept
}

fn kept_len(out: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let written: serde_json::Value = serde_json::from_slice(&std::fs::read(out)?)?;
    Ok(written["frames"].as_array().map_or(0, Vec::len))
}

#[derive(Clone, Copy)]
enum Clock {
    Fixed(i64),
    Live,
}

/// What one frame reads, on connections of its own.
struct Reader<'a> {
    db: &'a Path,
    state: &'a AppState,
    clock: Clock,
    days: u32,
    watch: &'a [String],
    legacy: &'a [String],
}

impl Reader<'_> {
    fn page(
        &self,
        search: &str,
        after: Option<&str>,
    ) -> Result<SessionPage, Box<dyn std::error::Error>> {
        let query = SessionQuery {
            search,
            after,
            ..Default::default()
        };
        Ok(match self.clock {
            Clock::Live => self.state.sessions_query(query, self.days)?,
            Clock::Fixed(at) => {
                let metrics = xt_metrics::MetricsDb::open(self.db)?;
                xtrace_desktop::dto::session_page(
                    &metrics,
                    self.days,
                    at,
                    jiff::tz::TimeZone::UTC,
                    MetricClock::System,
                    query,
                )?
            }
        })
    }

    fn dashboard(
        &self,
    ) -> Result<xtrace_desktop::dto::DashboardMetrics, Box<dyn std::error::Error>> {
        Ok(match self.clock {
            Clock::Live => self.state.metrics_dashboard(self.days)?,
            Clock::Fixed(at) => {
                // As the app's command prepares it: the saved typing speed,
                // saved break length and bundled price catalog.
                let store = Store::open(self.db)?;
                let typing =
                    xt_metrics::TypingRate::new(store.typing_speed()?.characters_per_minute())?;
                let break_length = xt_metrics::BreakLength::new(store.human_break()?.minutes())?;
                let metrics = xt_metrics::MetricsDb::open(self.db)?;
                xtrace_desktop::dashboard::assemble(
                    &metrics,
                    self.days,
                    at,
                    jiff::tz::TimeZone::UTC,
                    MetricClock::System,
                    &xt_metrics::PriceCatalog::bundled()?,
                    typing,
                    break_length,
                )?
            }
        })
    }

    fn frame(
        &self,
        at: &str,
        progress: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        // The whole list, every page, as raw membership: every field but the
        // display fields this work adds, digested.
        let (mut rows, mut digest, mut after) =
            (0_usize, std::hash::DefaultHasher::new(), None::<String>);
        loop {
            let page = self.page("", after.as_deref())?;
            let mut value = serde_json::to_value(&page)?;
            rows += page.rows.len();
            strip(
                &mut value,
                &["child_check", "known_child", "parent", "referenced_parents"],
            );
            value.to_string().hash(&mut digest);
            after = page.next;
            if after.is_none() {
                break;
            }
        }
        let mut searched = BTreeMap::new();
        for id in self.watch {
            let mut page = serde_json::to_value(self.page(id, None)?)?;
            blank(&mut page);
            searched.insert(id.clone(), page);
        }
        let mut dashboard = serde_json::to_value(self.dashboard()?)?;
        blank(&mut dashboard);
        let legacy = if self.legacy.is_empty() {
            serde_json::Value::Null
        } else {
            let connection = rusqlite::Connection::open_with_flags(
                self.db,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            let mut states = BTreeMap::<String, String>::new();
            for chunk in self.legacy.chunks(xt_store::session_list::MAX_CONTEXT) {
                let ids: Vec<&str> = chunk.iter().map(String::as_str).collect();
                for row in xt_store::session_list::context(&connection, &ids)? {
                    states.insert(row.id.clone(), format!("{:?}", row.check).to_lowercase());
                }
            }
            let per: Vec<serde_json::Value> = self
                .legacy
                .iter()
                .map(|id| serde_json::json!([id, states.get(id).map_or("absent", String::as_str)]))
                .collect();
            let mut counts = BTreeMap::<&str, usize>::new();
            for id in self.legacy {
                *counts
                    .entry(states.get(id).map_or("absent", String::as_str))
                    .or_default() += 1;
            }
            serde_json::json!({"counts": counts, "states": per})
        };
        Ok(serde_json::json!({
            "at": at,
            "progress": progress,
            "legacy": legacy,
            "responses": {
                "rows": rows,
                "membership_digest": format!("{:016x}", digest.finish()),
                "searched": searched,
                "dashboard": dashboard,
            },
        }))
    }
}

/// Counts of one background pass: what it did, never what it read.
fn counts(progress: &SpawnProgress) -> serde_json::Value {
    serde_json::json!({
        "changed": progress.changed,
        "shown_changes": progress.shown_changes(),
        "decided": progress.decided,
        "advanced": progress.advanced(),
        "settled": progress.checks.as_ref().map(|checks| checks.settled),
        "looked": progress.checks.as_ref().map(|checks| checks.looked),
        "linked": progress.launches.as_ref().map(|launches| launches.linked),
        "children": progress.launches.as_ref().map(|launches| launches.children),
        "bash_children": progress.bash.as_ref().map(|bash| bash.children),
    })
}

/// Remove the named keys everywhere.
fn strip(value: &mut serde_json::Value, keys: &[&str]) {
    match value {
        serde_json::Value::Object(object) => {
            for key in keys {
                object.remove(*key);
            }
            object.values_mut().for_each(|value| strip(value, keys));
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|item| strip(item, keys)),
        _ => {}
    }
}

/// Blank every saved title, repository, branch, path, URL and text: what is
/// written is identities, states, counts, times and measurements.
fn blank(value: &mut serde_json::Value) {
    const BLANKED: [&str; 9] = [
        "title",
        "repo",
        "repository",
        "branch",
        "cwd",
        "url",
        "path",
        "text",
        "data_dir",
    ];
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                if BLANKED.contains(&key.as_str()) && value.is_string() {
                    *value = serde_json::Value::Null;
                } else {
                    blank(value);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(blank),
        _ => {}
    }
}

/// Refuse a database in the app's own data folder (or aliased to it), a
/// sidecar alias, and the real home, before anything is opened.
fn guard(db: &Path, home: &Path) -> Result<(), Box<dyn std::error::Error>> {
    validate_index_destination(db, home)?;
    let real = env::var_os("HOME").map(PathBuf::from);
    if let Some(real) = &real
        && resolved(real).is_ok_and(|real| real == home || real.starts_with(home))
    {
        return Err("refusing the real home: run on a disposable copy".into());
    }
    let live: Vec<PathBuf> = real
        .into_iter()
        .map(|base| base.join("Library/Application Support/ai.xtrace.desktop"))
        .map(|live| resolved(&live).unwrap_or(live))
        .collect();
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let mut path = db.as_os_str().to_os_string();
        path.push(suffix);
        let target = resolved(Path::new(&path))?;
        if live.iter().any(|live| target.starts_with(live)) {
            return Err("refusing the app's own index: run on a disposable copy".into());
        }
    }
    Ok(())
}

/// `path` with every alias resolved; a name that does not exist yet is
/// resolved through its folder. A dangling alias is an error.
fn resolved(path: &Path) -> std::io::Result<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(path).is_ok() {
                return Err(error);
            }
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let name = path.file_name().ok_or(error)?;
            Ok(resolved(parent)?.join(name))
        }
        Err(error) => Err(error),
    }
}

fn now_ms() -> i64 {
    jiff::Timestamp::now().as_millisecond()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fixed_dashboard_uses_the_saved_break_instead_of_the_default() {
        let root = tempfile::TempDir::new().unwrap();
        let home = root.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let state = AppState::build(
            StartupOptions {
                data_dir: Some(root.path().join("data")),
                native_home: Some(home),
                ..Default::default()
            },
            || panic!("explicit test data directory"),
            || panic!("explicit test home"),
        )
        .unwrap();
        let db = root.path().join("data/xtrace.db");
        let at = 1_788_825_600_000;
        let mut store = Store::open(&db).unwrap();
        store
            .upsert_session(
                &xt_store::SessionMeta::new("work", "claude", xt_store::SessionSource::Fixture),
                false,
            )
            .unwrap();
        let records: Vec<xt_store::CanonicalRecord> = [0, 40]
            .into_iter()
            .enumerate()
            .map(|(index, minutes)| {
                serde_json::from_value(json!({
                    "uuid": format!("message-{index}"),
                    "type": "user",
                    "timestamp": jiff::Timestamp::from_millisecond(at - 3_600_000 + minutes * 60_000).unwrap().to_string(),
                    "message": {"role": "user", "content": [{"type": "text", "text": "x"}]}
                }))
                .unwrap()
            })
            .collect();
        store.upsert_records("work", &records, false).unwrap();
        let reader = Reader {
            db: &db,
            state: &state,
            clock: Clock::Fixed(at),
            days: 7,
            watch: &[],
            legacy: &[],
        };
        state.set_human_break(45).unwrap();
        let longer = reader.dashboard().unwrap();
        assert_eq!(longer.human_hours.current.break_minutes, 45);
        assert_eq!(longer.human_hours.current.active_ms, Some(40 * 60_000));
        state.set_human_break(30).unwrap();
        let shorter = reader.dashboard().unwrap();
        assert_eq!(shorter.human_hours.current.break_minutes, 30);
        assert_eq!(shorter.human_hours.current.active_ms, Some(0));
        state.shutdown();
    }

    #[test]
    fn equal_responses_keep_later_legacy_and_progress_changes() {
        let frames = vec![
            json!({"at":"one","responses":{"rows":1},"legacy":{"checked":0},"progress":{"settled":0}}),
            json!({"at":"two","responses":{"rows":1},"legacy":{"checked":1},"progress":{"settled":0}}),
            json!({"at":"three","responses":{"rows":1},"legacy":{"checked":1},"progress":{"settled":1}}),
            json!({"at":"four","responses":{"rows":1},"legacy":{"checked":1},"progress":{"settled":1}}),
        ];
        let kept = coalesce(frames);
        assert_eq!(kept.len(), 3);
        assert_eq!(kept[1]["legacy"]["checked"], 1);
        assert_eq!(kept[2]["progress"]["settled"], 1);
        assert_eq!(kept[2]["repeats"], 1);
    }

    #[test]
    fn repository_values_are_blank_without_changing_shape() {
        let mut value = json!({"session_pr_links":[{"repository":"private/repo","url":"https://example.test"}],"repo":"private"});
        blank(&mut value);
        assert_eq!(
            value["session_pr_links"][0]["repository"],
            serde_json::Value::Null
        );
        assert_eq!(value["session_pr_links"][0]["url"], serde_json::Value::Null);
        assert_eq!(value["repo"], serde_json::Value::Null);
    }
}
