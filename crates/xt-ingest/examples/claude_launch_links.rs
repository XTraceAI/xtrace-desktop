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
//! ```
//!
//! `--child` only reports; it never selects, proves or writes anything.
//!
//! Prints one JSON object: whether the watcher became ready and went quiet,
//! each host's scan status and session count, the launch summary (relations
//! by state, candidates by state, history files tracked and bytes covered),
//! how many relation changes the watcher announced, and, for `--child`, that
//! child's relation kind, state and parent identifiers. Never a path, prompt,
//! command or transcript text.
use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use xt_ingest::native::{
    ProducerSource,
    claude_launch::{LaunchBacklog, LaunchLimits, continue_claude_launches},
    session_creation::spawn_limits,
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

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}
