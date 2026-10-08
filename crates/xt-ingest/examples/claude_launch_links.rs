//! Run the automatic Claude launch links on a supplied local index until
//! the work is quiet, then print metadata-only counts. Intended for offline
//! acceptance on a copied index; native source files are only read.
//!
//! ```text
//! # The ordinary native watcher (scans every host, then its background passes):
//! cargo run -p xt-ingest --example claude_launch_links -- \
//!     --db INDEX --home HOME --pin PIN --producer-root PLUGIN_DIRECTORY \
//!     [--child CLAUDE_SESSION_UUID] [--timeout-secs N]
//! # Only the production launch drainer the watcher runs, over what the index
//! # already holds (startup sweep of every indexed parent), with no re-import:
//! cargo run -p xt-ingest --example claude_launch_links -- \
//!     --db INDEX --home HOME --backlog-only \
//!     [--child CLAUDE_SESSION_UUID] [--timeout-secs N]
//! # Every background child pass the watcher runs, over what the index already
//! # holds, with no re-import: the Codex typed-header pass and sweep, the Codex
//! # launch drainer and the Claude `Bash` launch sweep:
//! cargo run -p xt-ingest --example claude_launch_links -- \
//!     --db INDEX --home HOME --children \
//!     [--child SESSION_UUID]... [--timeout-secs N]
//! ```
//!
//! `--child` (repeatable with `--children`) only reports; it never selects,
//! proves or writes anything. `--children` refuses an index or sidecar that
//! is, or aliases, one inside the app's own data folder, before opening it:
//! it is for a disposable copy.
//!
//! Prints one JSON object: whether the watcher became ready and went quiet,
//! each host's scan status and session count, the launch summary (relations
//! by state, candidates by state, history files tracked and bytes covered),
//! how many relation changes the watcher announced, and, for `--child`, that
//! child's relation kind, state and parent identifiers. With `--children`,
//! each named session's child facts, relation, the structural identifiers of
//! a launch relation of either kind (a Claude `--session-id` or a Codex
//! `codex exec --json` launch) and the launch rows naming it. Never a path,
//! prompt, command or transcript text.
use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use xt_ingest::native::{
    ProducerSource,
    claude_launch::{LaunchBacklog, LaunchLimits, continue_claude_launches},
    session_creation::{SpawnBacklog, continue_codex_spawns, spawn_limits},
    validate_index_destination,
    watch::{ProbePoint, TailEvent, Tailer, WatchConfig},
};
use xt_store::{Host, Store};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let usage = "usage: claude_launch_links --db INDEX --home HOME \
                 (--pin PIN --producer-root PLUGIN_DIRECTORY | --backlog-only) \
                 [--child UUID] [--timeout-secs N]";
    let (Some(db), Some(home)) = (value("--db"), value("--home")) else {
        return Err(usage.into());
    };
    let backlog_only = args.iter().any(|arg| arg == "--backlog-only");
    if args.iter().any(|arg| arg == "--children") {
        return children(&args);
    }
    let child = value("--child");
    let timeout = Duration::from_secs(
        value("--timeout-secs")
            .map(|secs| secs.parse())
            .transpose()?
            .unwrap_or(1800),
    );
    let (db, home) = (PathBuf::from(db), PathBuf::from(home));
    validate_index_destination(&db, &home)?;
    let started = Instant::now();
    let (became_ready, went_quiet, hosts, announced, passes) = if backlog_only {
        let mut store = Store::open(&db)?;
        let mut backlog = LaunchBacklog::starting();
        let limits =
            LaunchLimits::remaining(spawn_limits().max_batch_bytes, spawn_limits().deadline);
        let (mut count, mut total, mut max) = (0_u64, 0_u64, 0_u64);
        let (mut entries, mut max_entries, mut idle) = (0_u64, 0_u64, 0_u32);
        let mut sum = xt_ingest::native::claude_launch::LaunchProgress::default();
        // Work left only on unchanged unfinished files or timed retries does
        // not advance: after 20 such passes in a row it is left, as the
        // watcher would back off.
        while backlog.pending() && started.elapsed() < timeout && idle < 20 {
            let pass =
                continue_claude_launches(&mut store, &home, &mut backlog, &limits, None, now_ms())?;
            count += 1;
            total += pass.bytes_read;
            max = max.max(pass.bytes_read);
            entries += pass.entries;
            max_entries = max_entries.max(pass.entries);
            sum.threads_unfinished += pass.threads_unfinished;
            sum.unfinished += pass.unfinished;
            sum.files_read += pass.files_read;
            sum.files_reused += pass.files_reused;
            sum.threads_unchanged += pass.threads_unchanged;
            sum.threads_busy += pass.threads_busy;
            sum.threads_waiting += pass.threads_waiting;
            sum.published_valid += pass.published_valid;
            sum.published_invalid += pass.published_invalid;
            sum.launches_found += pass.launches_found;
            sum.launches_broken += pass.launches_broken;
            sum.linked += pass.linked;
            sum.waiting += pass.waiting;
            sum.retry += pass.retry;
            sum.rejected += pass.rejected;
            sum.changed += pass.changed;
            if pass.advanced {
                idle = 0;
            } else {
                // Only waiting on retries: the watcher would back off here.
                idle += 1;
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        let quiet = !backlog.pending();
        (
            serde_json::Value::Null,
            quiet,
            serde_json::Value::Null,
            sum.changed,
            serde_json::json!({"passes": count, "bytes_read": total, "max_bytes_per_pass": max,
                "limit_bytes_per_pass": limits.max_pass_bytes,
                "files_read": sum.files_read, "files_reused": sum.files_reused,
                "threads_unchanged": sum.threads_unchanged, "threads_busy": sum.threads_busy,
                "threads_waiting": sum.threads_waiting,
                "published_valid": sum.published_valid, "published_invalid": sum.published_invalid,
                "launches_found": sum.launches_found, "launches_broken": sum.launches_broken,
                "linked": sum.linked, "waiting": sum.waiting, "retry": sum.retry,
                "rejected": sum.rejected, "threads_unfinished": sum.threads_unfinished,
                "children_unfinished": sum.unfinished, "entries": entries,
                "max_entries_per_pass": max_entries,
                "limit_entries_per_pass": limits.max_pass_entries,
                "left_idle": idle >= 20}),
        )
    } else {
        let (Some(pin), Some(root)) = (value("--pin"), value("--producer-root")) else {
            return Err(usage.into());
        };
        let events = Arc::new(Mutex::new(Vec::<usize>::new()));
        let ready = Arc::new(Mutex::new(None::<serde_json::Value>));
        let quiet = Arc::new(Mutex::new(false));
        let sink = {
            let (events, ready) = (Arc::clone(&events), Arc::clone(&ready));
            Box::new(move |event: TailEvent| match event {
                TailEvent::SessionCreationsChanged { changed } => {
                    events.lock().unwrap().push(changed);
                }
                TailEvent::Ready(readiness) => {
                    let hosts: Vec<serde_json::Value> = readiness
                        .report
                        .hosts
                        .iter()
                        .map(|host| {
                            serde_json::json!({"host": host.host, "status": host.status,
                            "sessions": host.sessions.len()})
                        })
                        .collect();
                    *ready.lock().unwrap() = Some(serde_json::json!(hosts));
                }
                _ => {}
            })
        };
        let probe = {
            let quiet = Arc::clone(&quiet);
            Arc::new(move |point: ProbePoint<'_>| {
                if let ProbePoint::SpawnsContinued { pending } = point {
                    *quiet.lock().unwrap() = !pending;
                }
            })
        };
        let tailer = Tailer::start(
            Store::open(&db)?,
            WatchConfig {
                home: home.clone(),
                hosts: vec![Host::Claude, Host::Codex, Host::Cursor],
                producer: ProducerSource::Checkout {
                    pin: PathBuf::from(pin),
                    plugin_root: Some(PathBuf::from(root)),
                },
                python: None,
                debounce: Duration::from_millis(250),
                spawn_limits: spawn_limits(),
                probe: Some(probe),
            },
            sink,
        );
        let became_ready = tailer.wait_ready(timeout).is_some();
        while became_ready && !*quiet.lock().unwrap() && started.elapsed() < timeout {
            std::thread::sleep(Duration::from_millis(200));
        }
        let went_quiet = *quiet.lock().unwrap();
        tailer.stop();
        let hosts = ready.lock().unwrap().clone().unwrap_or_default();
        let announced = events.lock().unwrap().iter().sum::<usize>();
        (
            serde_json::json!(became_ready),
            went_quiet,
            hosts,
            announced,
            serde_json::Value::Null,
        )
    };
    let store = Store::open(&db)?;
    let summary = store.claude_launch_summary()?;
    let child = match child {
        None => serde_json::Value::Null,
        Some(native) => {
            let sessions = store.user_sessions_with_native(Host::Claude, &native)?;
            let relation = match sessions.as_slice() {
                [one] => store.session_creation(one)?.map(|(proof, conflicted)| {
                    serde_json::json!({"evidence_kind": proof.evidence_kind,
                        "conflicted": conflicted,
                        "parent_native_session_id": proof.parent_native_session_id})
                }),
                _ => None,
            };
            let launch = match sessions.as_slice() {
                [one] => store.claude_launch_creation(one)?.map(|(proof, _)| {
                    serde_json::json!({"parent_session_id": proof.parent_session_id,
                        "first_record_uuid": proof.first_record_uuid,
                        "launch_call_id": proof.launch_call_id,
                        "launch_operation_index": proof.launch_operation_index,
                        "acknowledgment_call_id": proof.acknowledgment_call_id,
                        "acknowledgment_operation_index": proof.acknowledgment_operation_index,
                        "process_session_id": proof.process_session_id,
                        "segment_rollout_id": proof.segment_rollout_id,
                        "evidence_version": proof.evidence_version})
                }),
                _ => None,
            };
            let candidates: Vec<serde_json::Value> = store
                .claude_launch_candidates_for_child(&native, None, 100)?
                .into_iter()
                .map(|row| {
                    serde_json::json!({"launch_call_id": row.candidate.key.launch_call_id,
                        "rollout_id": row.candidate.key.rollout_id,
                        "source_revision": row.source_revision,
                        "source_verdict": row.source_verdict,
                        "child_state": row.child_state})
                })
                .collect();
            serde_json::json!({"indexed_sessions": sessions.len(), "relation": relation,
                "launch": launch, "candidates": candidates})
        }
    };
    println!(
        "{}",
        serde_json::json!({
            "mode": if backlog_only { "backlog_only" } else { "watcher" },
            "ready": became_ready,
            "quiet": went_quiet,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "hosts": hosts,
            "announced_changes": announced,
            "passes": passes,
            "summary": summary,
            "child": child,
        })
    );
    Ok(())
}

/// The `--children` mode: every background child pass, then metadata-only
/// counts and the named sessions' child status.
fn children(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let value = |flag: &str| {
        args.iter()
            .position(|arg| arg == flag)
            .and_then(|at| args.get(at + 1))
            .cloned()
    };
    let (Some(db), Some(home)) = (value("--db"), value("--home")) else {
        return Err(
            "usage: claude_launch_links --db INDEX --home HOME --children \
                    [--child SESSION_UUID]... [--timeout-secs N]"
                .into(),
        );
    };
    let named: Vec<String> = args
        .windows(2)
        .filter(|pair| pair[0] == "--child")
        .map(|pair| pair[1].clone())
        .collect();
    let timeout = Duration::from_secs(
        value("--timeout-secs")
            .map(|secs| secs.parse())
            .transpose()?
            .unwrap_or(1800),
    );
    let home = PathBuf::from(home);
    let db = guarded(&PathBuf::from(db), &home)?;
    let started = Instant::now();
    let mut store = Store::open(&db)?;
    let mut spawns = SpawnBacklog::starting();
    let (mut passes, mut bytes, mut idle) = (0_u64, 0_u64, 0_u32);
    let mut sums = serde_json::Map::new();
    let mut add = |name: &str, value: usize| {
        let entry = sums.entry(name.to_owned()).or_insert(serde_json::json!(0));
        *entry = serde_json::json!(entry.as_u64().unwrap_or(0) + value as u64);
    };
    while spawns.pending() && started.elapsed() < timeout && idle < 20 {
        let pass = continue_codex_spawns(
            &mut store,
            &home,
            &mut spawns,
            spawn_limits(),
            None,
            now_ms(),
        )?;
        passes += 1;
        bytes += pass.bytes_read;
        add("changed", pass.changed);
        add("codex_spawned", pass.spawned);
        add("codex_bootstrapped", pass.bootstrapped);
        add("codex_swept", pass.swept);
        if let Some(launches) = &pass.launches {
            add("launch_children", launches.children);
            add("launch_unparented", launches.unparented);
            add("launch_linked", launches.linked);
            add("launch_waiting", launches.waiting);
            add("launch_retry", launches.retry);
            add("launch_rejected", launches.rejected);
            add("launch_found", launches.launches_found);
        }
        if let Some(bash) = &pass.bash {
            add("bash_callers_read", bash.callers_read);
            add("bash_callers_unreadable", bash.callers_unreadable);
            add("bash_launches", bash.launches);
            add("bash_children", bash.children);
            add("bash_already", bash.already);
            add("bash_ambiguous", bash.ambiguous);
            add("bash_rejected", bash.rejected);
            add("bash_waiting", bash.waiting);
        }
        if pass.advanced() {
            idle = 0;
        } else {
            idle += 1;
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    let quiet = !spawns.pending();
    let connection =
        rusqlite::Connection::open_with_flags(&db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut sessions = Vec::new();
    let mut launch_rows = Vec::new();
    for native in &named {
        for host in [Host::Claude, Host::Codex] {
            for id in store.user_sessions_with_native(host, native)? {
                let facts: Vec<serde_json::Value> = store
                    .child_facts(&id)?
                    .into_iter()
                    .map(|(fact, accepted)| {
                        serde_json::json!({"kind": fact.evidence_kind,
                            "version": fact.evidence_version, "accepted": accepted,
                            "source_native_session_id": fact.source_native_session_id,
                            "source_rollout_id": fact.source_rollout_id,
                            "launch_call_id": fact.launch_call_id,
                            "launch_operation_index": fact.launch_operation_index,
                            "first_record_uuid": fact.first_record_uuid})
                    })
                    .collect();
                let launch = match host {
                    Host::Codex => store.codex_cli_launch_creation(&id)?,
                    _ => store.claude_launch_creation(&id)?,
                }
                .map(|(proof, conflicted)| {
                    serde_json::json!({"conflicted": conflicted,
                        "parent_session_id": proof.parent_session_id,
                        "first_record_uuid": proof.first_record_uuid,
                        "launch_call_id": proof.launch_call_id,
                        "launch_operation_index": proof.launch_operation_index,
                        "acknowledgment_call_id": proof.acknowledgment_call_id,
                        "process_session_id": proof.process_session_id,
                        "segment_rollout_id": proof.segment_rollout_id,
                        "launch_ordinal": proof.launch_ordinal,
                        "acknowledgment_ordinal": proof.acknowledgment_ordinal,
                        "evidence_version": proof.evidence_version})
                });
                let context = xt_store::session_list::context(&connection, &[&id])?;
                let relation = store.session_creation(&id)?.map(|(proof, conflicted)| {
                    serde_json::json!({"evidence_kind": proof.evidence_kind,
                        "evidence_version": proof.evidence_version, "conflicted": conflicted,
                        "parent_native_session_id": proof.parent_native_session_id})
                });
                sessions.push(serde_json::json!({"native": native, "host": host,
                    "known_child": context.first().map(|c| c.known_child),
                    "parent_session_id": context.first()
                        .and_then(|c| c.parent.as_ref()).map(|p| p.session_id.clone()),
                    "relation": relation, "launch": launch, "facts": facts}));
            }
        }
        let candidates: Vec<serde_json::Value> = store
            .claude_launch_candidates_for_child(native, None, 100)?
            .into_iter()
            .map(|row| {
                serde_json::json!({"native": native, "child_host": row.candidate.child_host,
                    "parent_native_session_id": row.candidate.key.parent_native_session_id,
                    "rollout_id": row.candidate.key.rollout_id,
                    "launch_call_id": row.candidate.key.launch_call_id,
                    "launch_operation_index": row.candidate.key.launch_operation_index,
                    "acknowledgment_call_id": row.candidate.acknowledgment_call_id,
                    "process_session_id": row.candidate.process_session_id,
                    "source_revision": row.source_revision,
                    "source_verdict": row.source_verdict,
                    "child_state": row.child_state})
            })
            .collect();
        launch_rows.extend(candidates);
    }
    println!(
        "{}",
        serde_json::json!({
            "mode": "children",
            "quiet": quiet,
            "left_idle": idle >= 20,
            "elapsed_ms": started.elapsed().as_millis() as u64,
            "passes": passes,
            "bytes_read": bytes,
            "counts": sums,
            "child_facts": store.child_fact_summary()?,
            "launch_summary": store.claude_launch_summary()?,
            "sessions": sessions,
            "launch_rows": launch_rows,
        })
    );
    Ok(())
}

/// Check a `--children` index before anything opens it, and give the path to
/// open it at. The index and every sidecar SQLite may use (`-wal`, `-shm`,
/// `-journal`) are resolved through any alias, under the name given and
/// under the resolved name; none may lie in the app's own data folder, and
/// one that cannot be resolved (a dangling alias) is refused. The ordinary
/// destination check, which refuses hard links and native-source aliases,
/// runs on both names too. The store is then opened at the resolved name, so
/// the name that was checked is the one opened.
fn guarded(db: &std::path::Path, home: &std::path::Path) -> Result<PathBuf, &'static str> {
    const UNVERIFIED: &str = "cannot verify the index or a sidecar: run on a disposable copy";
    let resolved_db = resolved(db).map_err(|_| UNVERIFIED)?;
    validate_index_destination(db, home)?;
    validate_index_destination(&resolved_db, home)?;
    let lives: Vec<PathBuf> = [
        Some(home.to_path_buf()),
        env::var_os("HOME").map(PathBuf::from),
    ]
    .into_iter()
    .flatten()
    // The folder of the earlier app ID still holds real data too.
    .flat_map(|base| {
        ["ai.xtrace.app", "ai.xtrace.desktop"]
            .map(|id| base.join("Library/Application Support").join(id))
    })
    .map(|live| resolved(&live).unwrap_or(live))
    .collect();
    for name in [db, resolved_db.as_path()] {
        for suffix in ["", "-wal", "-shm", "-journal"] {
            let mut path = name.as_os_str().to_os_string();
            path.push(suffix);
            let target = resolved(std::path::Path::new(&path)).map_err(|_| UNVERIFIED)?;
            if lives.iter().any(|live| target.starts_with(live)) {
                return Err("refusing the app's own index: run on a disposable copy");
            }
        }
    }
    Ok(resolved_db)
}

/// `path` with every alias resolved; a name that does not exist yet is
/// resolved through its folder. A dangling alias is an error.
fn resolved(path: &std::path::Path) -> std::io::Result<PathBuf> {
    match path.canonicalize() {
        Ok(path) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if std::fs::symlink_metadata(path).is_ok() {
                return Err(error);
            }
            let parent = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(std::path::Path::new("."));
            let name = path.file_name().ok_or(error)?;
            Ok(resolved(parent)?.join(name))
        }
        Err(error) => Err(error),
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink};

    /// A disposable fake home whose app-data folder holds a fake live index
    /// and its sidecars, and a separate disposable work folder. Nothing here
    /// names, opens or aliases the real app data.
    struct Fake {
        _temp: tempfile::TempDir,
        home: PathBuf,
        live: PathBuf,
        work: PathBuf,
    }

    fn fake() -> Fake {
        let temp = tempfile::TempDir::new().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let home = root.join("home");
        let live = home.join("Library/Application Support/ai.xtrace.app");
        let work = root.join("work");
        fs::create_dir_all(&live).unwrap();
        fs::create_dir_all(&work).unwrap();
        for name in ["index.sqlite", "index.sqlite-wal", "index.sqlite-shm"] {
            fs::write(live.join(name), b"fake live").unwrap();
        }
        Fake {
            _temp: temp,
            home,
            live,
            work,
        }
    }

    /// The fake live files, byte for byte, and nothing added beside them.
    fn untouched(fake: &Fake) {
        let mut names: Vec<_> = fs::read_dir(&fake.live)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            ["index.sqlite", "index.sqlite-shm", "index.sqlite-wal"]
        );
        for name in &names {
            assert_eq!(fs::read(fake.live.join(name)).unwrap(), b"fake live");
        }
    }

    #[test]
    fn an_alias_of_the_live_index_or_its_sidecars_is_refused_before_opening() {
        // A plain disposable copy passes, and opens where it lies.
        let f = fake();
        let copy = f.work.join("copy.sqlite");
        fs::write(&copy, b"").unwrap();
        assert_eq!(guarded(&copy, &f.home).unwrap(), copy);

        // The live index itself, inside the live folder.
        assert!(guarded(&f.live.join("index.sqlite"), &f.home).is_err());

        // A symlink elsewhere to the live index.
        let alias = f.work.join("alias.sqlite");
        symlink(f.live.join("index.sqlite"), &alias).unwrap();
        assert!(guarded(&alias, &f.home).is_err(), "database alias");

        // A symlinked folder that is the live folder.
        let folder = f.work.join("folder");
        symlink(&f.live, &folder).unwrap();
        assert!(guarded(&folder.join("index.sqlite"), &f.home).is_err());

        // A plain copy whose sidecar is a symlink to a live sidecar.
        for (suffix, target) in [
            ("-wal", "index.sqlite-wal"),
            ("-shm", "index.sqlite-shm"),
            ("-journal", "index.sqlite-wal"),
        ] {
            let f = fake();
            let copy = f.work.join("copy.sqlite");
            fs::write(&copy, b"").unwrap();
            symlink(
                f.live.join(target),
                f.work.join(format!("copy.sqlite{suffix}")),
            )
            .unwrap();
            assert!(guarded(&copy, &f.home).is_err(), "{suffix} alias");
            untouched(&f);
        }
        // A sidecar symlink to a live sidecar not created yet.
        let f = fake();
        fs::remove_file(f.live.join("index.sqlite-shm")).unwrap();
        let copy = f.work.join("copy.sqlite");
        fs::write(&copy, b"").unwrap();
        symlink(
            f.live.join("index.sqlite-shm"),
            f.work.join("copy.sqlite-shm"),
        )
        .unwrap();
        assert!(guarded(&copy, &f.home).is_err(), "dangling sidecar alias");
        assert!(!f.live.join("index.sqlite-shm").exists());

        // A hard link of the live index is still refused.
        let f = fake();
        let linked = f.work.join("linked.sqlite");
        fs::hard_link(f.live.join("index.sqlite"), &linked).unwrap();
        assert!(guarded(&linked, &f.home).is_err(), "hard link");
        untouched(&f);
    }
}
