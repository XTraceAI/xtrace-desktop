use super::*;
use serde_json::{Value, json};

const NATIVE: &str = "11111111-1111-4111-8111-111111111111";
const CHILD: &str = "22222222-2222-4222-8222-222222222222";

fn ids() -> Vec<String> {
    vec![format!("codex-{NATIVE}")]
}
fn eligible() -> BTreeSet<String> {
    ids().into_iter().collect()
}
fn register(service: &LiveCodexStatus) -> String {
    let result = service.read(&[], None, BTreeSet::new()).unwrap();
    assert!(result.states.is_empty());
    result.view_id
}
fn change(value: Value) -> protocol::Change {
    serde_json::from_value(value).unwrap()
}
fn snapshot(native: &str, revision: u64, runtime: Value) -> Value {
    json!({"type":"snapshot","revision":revision,
        "conversationState":{"id":native,"hostId":"local","threadRuntimeStatus":runtime,
            "turns":[{"content":"TRANSIENT PRIVATE CONTENT MUST NOT BE RETAINED"}]}})
}
fn active(flags: &[&str]) -> Value {
    json!({"type":"active","activeFlags":flags})
}
fn patches(base: u64, revision: u64, patches: Value) -> Value {
    json!({"type":"patches","baseRevision":base,"revision":revision,"patches":patches})
}

#[test]
fn live_exact_canonical_identity() {
    assert_eq!(canonical_native(&ids()[0]), Some(NATIVE));
    for id in [
        NATIVE.to_owned(),
        format!("codex-{NATIVE}-child"),
        format!("claude-{NATIVE}"),
        "codex-AAAAAAAA-1111-4111-8111-111111111111".into(),
        "codex-../ipc".into(),
    ] {
        assert_eq!(canonical_native(&id), None, "{id}");
    }
}

#[test]
fn live_exact_runtime_mapping_and_unknown_flags_fail_closed() {
    let mut thread = protocol::ThreadStatus::default();
    for (runtime, expected) in [
        (active(&[]), LiveSessionState::Running),
        (
            active(&["waitingOnUserInput"]),
            LiveSessionState::WaitingInput,
        ),
        (
            active(&["waitingOnApproval"]),
            LiveSessionState::WaitingApproval,
        ),
        (
            active(&["waitingOnUserInput", "waitingOnApproval"]),
            LiveSessionState::WaitingApproval,
        ),
        (json!({"type":"idle"}), LiveSessionState::Idle),
        (json!({"type":"systemError"}), LiveSessionState::Unknown),
        (json!({"type":"notLoaded"}), LiveSessionState::Unknown),
        (json!({"type":"active"}), LiveSessionState::Unknown),
        (active(&["futureFlag"]), LiveSessionState::Unknown),
        (
            active(&["waitingOnApproval", "waitingOnApproval"]),
            LiveSessionState::Unknown,
        ),
        (
            json!({"type":"idle","activeFlags":["futureFlag"]}),
            LiveSessionState::Unknown,
        ),
    ] {
        thread.apply(NATIVE, change(snapshot(NATIVE, 1, runtime)));
        assert_eq!(thread.status(), expected);
    }
}

#[test]
fn live_revision_gaps_identity_and_malformed_changes_require_snapshot() {
    let mut thread = protocol::ThreadStatus::default();
    let invalid = [
        snapshot(CHILD, 2, active(&[])),
        snapshot(NATIVE, 1 << 53, active(&[])),
        patches(0, 2, json!([])),
        patches(1, 1, json!([])),
        patches(1, 1 << 53, json!([])),
        patches(1, 2, json!([{"op":"replace","path":["id"],"value":NATIVE}])),
        patches(
            1,
            2,
            json!([{"op":"replace","path":["hostId"],"value":"remote"}]),
        ),
        patches(1, 2, json!([{"op":"move","path":["turns"]}])),
        patches(
            1,
            2,
            json!([{"op":"replace","path":["threadRuntimeStatus","future"],"value":true}]),
        ),
        json!({"type":"reset"}),
    ];
    for invalid in invalid {
        thread.reset();
        assert!(thread.apply(NATIVE, change(snapshot(NATIVE, 1, active(&[])))));
        assert!(!thread.apply(NATIVE, change(invalid)));
        assert_eq!(thread.status(), LiveSessionState::Unknown);
        assert!(!thread.apply(NATIVE, change(patches(1, 2, json!([])))));
        assert!(thread.apply(NATIVE, change(snapshot(NATIVE, 3, json!({"type":"idle"})))));
    }
    assert!(!thread.apply(NATIVE, change(snapshot(NATIVE, 2, active(&[])))));
}

#[test]
fn live_repeated_old_snapshots_and_invalidation_preserve_revision_floor() {
    let mut thread = protocol::ThreadStatus::default();
    assert!(thread.apply(NATIVE, change(snapshot(NATIVE, 3, json!({"type":"idle"})))));
    for _ in 0..2 {
        assert!(!thread.apply(NATIVE, change(snapshot(NATIVE, 2, active(&[])))));
        assert_eq!(thread.status(), LiveSessionState::Unknown);
        assert!(thread.needs_snapshot());
    }
    assert!(!thread.apply(NATIVE, change(patches(3, 4, json!([])))));
    assert!(thread.apply(NATIVE, change(snapshot(NATIVE, 3, json!({"type":"idle"})))));
    thread.clear();
    assert!(!thread.apply(NATIVE, change(snapshot(NATIVE, 2, active(&[])))));
    assert!(thread.apply(NATIVE, change(snapshot(NATIVE, 4, active(&[])))));
    thread.reset();
    assert!(thread.needs_snapshot());
    assert!(thread.apply(NATIVE, change(snapshot(NATIVE, 0, active(&[])))));
}

#[test]
fn live_root_replacement_and_nested_flag_edits() {
    let mut thread = protocol::ThreadStatus::default();
    assert!(thread.apply(NATIVE, change(snapshot(NATIVE, 0, active(&[])))));
    for (base, ops, expected) in [
        (
            0,
            json!([{"op":"add","path":["threadRuntimeStatus","activeFlags",0],"value":"waitingOnUserInput"}]),
            LiveSessionState::WaitingInput,
        ),
        (
            1,
            json!([{"op":"add","path":"/threadRuntimeStatus/activeFlags/-","value":"waitingOnApproval"}]),
            LiveSessionState::WaitingApproval,
        ),
        (
            2,
            json!([{"op":"remove","path":["threadRuntimeStatus","activeFlags",1]}]),
            LiveSessionState::WaitingInput,
        ),
        (
            3,
            json!([{"op":"remove","path":"/threadRuntimeStatus/activeFlags/0"}]),
            LiveSessionState::Running,
        ),
        (
            4,
            json!([{"op":"replace","path":["threadRuntimeStatus"],"value":{"type":"idle"}}]),
            LiveSessionState::Idle,
        ),
        (
            5,
            json!([{"op":"replace","path":[],"value":{"id":NATIVE,"hostId":"local","threadRuntimeStatus":active(&[])}}]),
            LiveSessionState::Running,
        ),
        (
            6,
            json!([{"op":"replace","path":"","value":{"id":NATIVE,"hostId":"local","threadRuntimeStatus":{"type":"idle"}}}]),
            LiveSessionState::Idle,
        ),
    ] {
        assert!(thread.apply(NATIVE, change(patches(base, base + 1, ops))));
        assert_eq!(thread.status(), expected);
    }
}

#[test]
fn live_parser_caps_depth_patch_count_and_drops_snapshot_content() {
    let body = json!({"type":"broadcast","version":11,"params":{
        "conversationId":NATIVE,"hostId":"local","change":snapshot(NATIVE, 0, active(&[]))}});
    let parsed = protocol::parse(&serde_json::to_vec(&body).unwrap()).unwrap();
    let projection = match parsed.params.unwrap().change.unwrap() {
        protocol::Change::Snapshot {
            conversation_state, ..
        } => conversation_state,
        _ => panic!("snapshot"),
    };
    assert_eq!(projection.id, NATIVE);
    assert_eq!(projection.thread_runtime_status, active(&[]));
    let too_deep = format!(
        "{{\"type\":\"broadcast\",\"discarded\":{}0{}}}",
        "[".repeat(130),
        "]".repeat(130)
    );
    assert!(protocol::parse(too_deep.as_bytes()).is_err());
    let too_many = json!({"type":"broadcast","params":{"change":patches(0, 1,
        Value::Array((0..257).map(|_| json!({"op":"remove","path":["turns",0]})).collect()))}});
    assert!(protocol::parse(&serde_json::to_vec(&too_many).unwrap()).is_err());
}

#[test]
fn live_optional_version_distinguishes_omission_from_null_and_malformed_values() {
    let mut response = json!({"type":"response","requestId":"request-1","method":"initialize","resultType":"success","result":{"clientId":"observer"}});
    assert_eq!(
        protocol::parse(&serde_json::to_vec(&response).unwrap())
            .unwrap()
            .version,
        None
    );
    for version in [json!(0), json!(1), json!(11)] {
        response["version"] = version.clone();
        assert_eq!(
            protocol::parse(&serde_json::to_vec(&response).unwrap())
                .unwrap()
                .version,
            version.as_u64()
        );
    }
    for version in [
        Value::Null,
        json!("0"),
        json!(-1),
        json!(0.5),
        json!(true),
        json!([]),
        json!({}),
    ] {
        response["version"] = version;
        assert!(protocol::parse(&serde_json::to_vec(&response).unwrap()).is_err());
    }
}

#[test]
fn live_no_native_home_does_not_start_worker_and_missing_tokens_are_refused() {
    let service = LiveCodexStatus::new(None);
    let view = register(&service);
    assert_eq!(
        service
            .read(&ids(), Some(&view), eligible())
            .unwrap()
            .states[0]
            .status,
        LiveSessionState::Unknown
    );
    assert!(service.worker.lock().unwrap().is_none());
    service.release("before-read").unwrap();
    assert!(
        service
            .read(&ids(), Some("before-read"), eligible())
            .is_err()
    );
    service.release(&view).unwrap();
    assert!(service.read(&ids(), Some(&view), eligible()).is_err());
    service.shutdown();
    assert!(service.read(&[], None, BTreeSet::new()).is_err());
}

#[test]
fn live_limits_lease_and_over_4096_release_cycles_remain_usable() {
    let service = LiveCodexStatus::new(None);
    let views: Vec<_> = (0..MAX_VIEWS).map(|_| register(&service)).collect();
    assert!(service.read(&[], None, BTreeSet::new()).is_err());
    {
        let mut hub = service.shared.hub.lock().unwrap();
        for view in hub.views.values_mut() {
            view.read_at = Instant::now() - LEASE;
        }
        hub.expire(Instant::now());
        assert!(hub.views.is_empty());
    }
    for view in views {
        assert!(service.read(&ids(), Some(&view), eligible()).is_err());
    }
    let mut issued = BTreeSet::new();
    for _ in 0..4100 {
        let view = register(&service);
        assert!(issued.insert(view.clone()));
        service.release(&view).unwrap();
        service.release(&view).unwrap();
        assert!(service.read(&ids(), Some(&view), eligible()).is_err());
    }
    let view = register(&service);
    service.read(&ids(), Some(&view), eligible()).unwrap();
    assert_eq!(service.shared.hub.lock().unwrap().views.len(), 1);
}

#[test]
fn live_registration_is_empty_and_tokens_are_rechecked_after_lookup() {
    let root = tempfile::TempDir::new().unwrap();
    let service = LiveCodexStatus::new(Some(root.path()));
    assert!(service.read(&ids(), None, eligible()).is_err());
    assert!(service.shared.hub.lock().unwrap().views.is_empty());
    let view = register(&service);
    assert!(service.requested_targets().is_empty());
    assert!(service.worker.lock().unwrap().is_none());
    assert!(
        service.shared.hub.lock().unwrap().views[&view]
            .targets
            .is_empty()
    );
    service.require_active(&view).unwrap(); // before metadata lookup
    let resolved = eligible();
    service.release(&view).unwrap(); // navigation while lookup is in flight
    assert!(service.read(&ids(), Some(&view), resolved).is_err());
    assert!(service.requested_targets().is_empty());
    let next = register(&service);
    assert_ne!(next, view);
    service.require_active(&next).unwrap();
    let resolved = eligible();
    service
        .shared
        .hub
        .lock()
        .unwrap()
        .views
        .get_mut(&next)
        .unwrap()
        .read_at = Instant::now() - LEASE;
    assert!(service.read(&ids(), Some(&next), resolved).is_err());
    assert!(service.require_active(&next).is_err());
    assert!(service.requested_targets().is_empty());
    assert!(service.worker.lock().unwrap().is_none());
    let fresh = register(&service);
    assert_ne!(fresh, next);
    assert!(service.require_active(&fresh).is_ok());
    assert!(service.read(&ids(), Some(&view), eligible()).is_err());
    assert_eq!(service.shared.hub.lock().unwrap().views.len(), 1);
}

#[test]
fn live_distinct_target_limit_is_shared_across_views() {
    let root = tempfile::TempDir::new().unwrap();
    let service = LiveCodexStatus::new(Some(root.path()));
    let first = register(&service);
    let second = register(&service);
    let targets: BTreeSet<_> = (0..16)
        .map(|n| format!("codex-{n:08x}-1111-4111-8111-111111111111"))
        .collect();
    let requested: Vec<_> = targets.iter().cloned().collect();
    service
        .read(&requested, Some(&first), targets.clone())
        .unwrap();
    service.read(&requested, Some(&second), targets).unwrap(); // shared identities count once
    assert_eq!(service.requested_targets().len(), MAX_TARGETS);
    assert!(service.read(&ids(), Some(&second), eligible()).is_err());
    assert_eq!(service.requested_targets().len(), MAX_TARGETS);
    service.release(&first).unwrap();
    service.read(&ids(), Some(&second), eligible()).unwrap();
    assert_eq!(service.requested_targets(), ids());
}

#[cfg(target_os = "macos")]
#[test]
fn live_mixed_saved_ids_do_not_alias_same_uuid_and_share_caps() {
    let root = tempfile::TempDir::new().unwrap();
    let service = LiveCodexStatus::new(Some(root.path()));
    let view = register(&service);
    let requested = vec![NATIVE.to_owned(), format!("codex-{NATIVE}")];
    let result = service
        .read_with(
            &requested,
            Some(&view),
            requested.iter().cloned().collect(),
            |_, targets, cancelled| {
                assert_eq!(targets, &[NATIVE.to_owned()].into());
                assert!(!cancelled());
                [(NATIVE.to_owned(), LiveSessionState::Running)].into()
            },
        )
        .unwrap();
    assert_eq!(result.states[0].status, LiveSessionState::Running);
    assert_eq!(result.states[1].status, LiveSessionState::Unknown);
    assert_eq!(service.requested_targets(), requested);
    let second = register(&service);
    let others: Vec<_> = (3..17)
        .map(|n| format!("{n:08x}-1111-4111-8111-111111111111"))
        .collect();
    service
        .read_with(
            &others,
            Some(&second),
            others.iter().cloned().collect(),
            |_, _, _| BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(service.requested_targets().len(), 16);
    let over_limit: Vec<_> = others.iter().cloned().chain([CHILD.to_owned()]).collect();
    assert!(
        service
            .read(
                &over_limit,
                Some(&second),
                over_limit.iter().cloned().collect()
            )
            .is_err()
    );
    service.release(&view).unwrap();
    service
        .read(
            &[CHILD.to_owned()],
            Some(&second),
            [CHILD.to_owned()].into(),
        )
        .unwrap();
    assert_eq!(service.requested_targets(), [CHILD]);
}

#[cfg(target_os = "macos")]
#[test]
fn live_claude_only_read_has_no_socket_worker_or_cached_running_claim() {
    let root = tempfile::TempDir::new().unwrap();
    let service = LiveCodexStatus::new(Some(root.path()));
    let view = register(&service);
    let requested = vec![NATIVE.to_owned()];
    let result = service
        .read_with(
            &requested,
            Some(&view),
            requested.iter().cloned().collect(),
            |_, _, _| [(NATIVE.to_owned(), LiveSessionState::Running)].into(),
        )
        .unwrap();
    assert_eq!(result.states[0].status, LiveSessionState::Running);
    assert!(service.worker.lock().unwrap().is_none());
    assert_eq!(
        service.shared.hub.lock().unwrap().targets[NATIVE].status,
        LiveSessionState::Unknown
    );
    assert_eq!(
        service
            .read(&requested, Some(&view), requested.iter().cloned().collect())
            .unwrap()
            .states[0]
            .status,
        LiveSessionState::Unknown
    ); // missing registry never retains previous busy
}

#[cfg(target_os = "macos")]
#[test]
fn live_claude_late_release_expiry_shutdown_and_replacement_drop_results() {
    for action in [
        "release",
        "expiry",
        "shutdown",
        "replace",
        "target-generation",
    ] {
        let root = tempfile::TempDir::new().unwrap();
        let service = LiveCodexStatus::new(Some(root.path()));
        let view = register(&service);
        let requested = vec![NATIVE.to_owned()];
        let result = service.read_with(
            &requested,
            Some(&view),
            requested.iter().cloned().collect(),
            |_, _, cancelled| {
                assert!(!cancelled());
                match action {
                    "release" => service.release(&view).unwrap(),
                    "expiry" => {
                        service
                            .shared
                            .hub
                            .lock()
                            .unwrap()
                            .views
                            .get_mut(&view)
                            .unwrap()
                            .read_at = Instant::now() - LEASE
                    }
                    "shutdown" => service.shutdown(),
                    "replace" => {
                        service.read(&[], Some(&view), BTreeSet::new()).unwrap();
                    }
                    _ => {
                        service
                            .shared
                            .hub
                            .lock()
                            .unwrap()
                            .targets
                            .get_mut(NATIVE)
                            .unwrap()
                            .generation += 1;
                    }
                }
                assert!(cancelled());
                [(NATIVE.to_owned(), LiveSessionState::Running)].into()
            },
        );
        assert!(result.is_err(), "{action}");
    }
}

#[cfg(target_os = "macos")]
#[test]
fn live_claude_scan_guard_is_nonblocking_and_fixture_mode_never_scans() {
    let root = tempfile::TempDir::new().unwrap();
    let service = LiveCodexStatus::new(Some(root.path()));
    let view = register(&service);
    let requested = vec![NATIVE.to_owned()];
    let _scan = service.claude_scan.lock().unwrap();
    let result = service
        .read_with(
            &requested,
            Some(&view),
            requested.iter().cloned().collect(),
            |_, _, _| panic!("second scan must not queue"),
        )
        .unwrap();
    assert_eq!(result.states[0].status, LiveSessionState::Unknown);
    let fixture = LiveCodexStatus::new(None);
    let view = register(&fixture);
    assert_eq!(
        fixture
            .read_with(
                &requested,
                Some(&view),
                requested.iter().cloned().collect(),
                |_, _, _| panic!("fixture must never inspect a home")
            )
            .unwrap()
            .states[0]
            .status,
        LiveSessionState::Unknown
    );
    assert!(fixture.requested_targets().is_empty());
}

/// A version-11 snapshot shaped like the observed producer, with `turns`
/// entries of ~64KiB text before the status fields. It is generated while
/// read, so neither the test nor the reader ever holds the whole body.
#[cfg(unix)]
fn large_snapshot(
    native: &str,
    turns: usize,
    runtime: &Value,
) -> (usize, impl std::io::Read + use<>) {
    use std::io::Read;
    struct Cycle {
        piece: Vec<u8>,
        left: usize,
        at: usize,
    }
    impl Read for Cycle {
        fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
            if self.left == 0 {
                return Ok(0);
            }
            let n = out.len().min(self.piece.len() - self.at);
            out[..n].copy_from_slice(&self.piece[self.at..self.at + n]);
            self.at += n;
            if self.at == self.piece.len() {
                self.at = 0;
                self.left -= 1;
            }
            Ok(n)
        }
    }
    let prefix = format!(
        "{{\"type\":\"broadcast\",\"method\":\"thread-stream-state-changed\",\"sourceClientId\":\"owner\",\
         \"version\":11,\"targetClientIds\":[\"observer\"],\"params\":{{\"conversationId\":\"{native}\",\
         \"hostId\":\"local\",\"change\":{{\"type\":\"snapshot\",\"revision\":3,\"conversationState\":{{\"turns\":["
    );
    let mut piece = serde_json::to_vec(&json!({"id":"turn","items":[{"type":"agentMessage",
        "text":format!("{} \u{e9}\u{1F600} \"quoted\" \\ \n", "x".repeat(64 * 1024))}]}))
    .unwrap();
    piece.push(b',');
    let last = br#"{"id":"turn"}"#;
    let suffix = format!(
        "],\"id\":\"{native}\",\"hostId\":\"local\",\"threadRuntimeStatus\":{runtime}}}}}}}}}"
    );
    let length = prefix.len() + piece.len() * turns + last.len() + suffix.len();
    let reader = std::io::Cursor::new(prefix.into_bytes())
        .chain(Cycle {
            piece,
            left: turns,
            at: 0,
        })
        .chain(last.as_slice())
        .chain(std::io::Cursor::new(suffix.into_bytes()));
    (length, reader)
}

#[cfg(unix)]
fn frame_of(length: usize, body: impl std::io::Read) -> impl std::io::Read {
    use std::io::Read;
    std::io::Cursor::new((length as u32).to_le_bytes()).chain(body)
}

#[cfg(unix)]
#[test]
fn live_large_snapshot_streams_with_bounded_memory_in_small_reads() {
    use socket::{CHUNK, Frame, Frames};
    use std::io::{self, Read};
    /// Small reads with a would-block between each, like a slow peer.
    struct Trickle<R> {
        inner: R,
        max: usize,
        blocked: bool,
    }
    impl<R: Read> Read for Trickle<R> {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            self.blocked = !self.blocked;
            if self.blocked {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let n = out.len().min(self.max);
            self.inner.read(&mut out[..n])
        }
    }
    for (runtime, expected) in [
        (
            active(&["waitingOnApproval"]),
            LiveSessionState::WaitingApproval,
        ),
        (active(&[]), LiveSessionState::Running),
    ] {
        // ~20.5MiB: above the old 16MiB cap and both observed snapshots.
        let (length, body) = large_snapshot(NATIVE, 320, &runtime);
        assert!(length > 20 * 1024 * 1024, "{length}");
        let mut stream = Trickle {
            inner: frame_of(length, body),
            max: 7 * 1024,
            blocked: false,
        };
        let mut frames = Frames::default();
        let (mut calls, mut peak) = (0, 0);
        let message = loop {
            calls += 1;
            let frame = frames.read(&mut stream).unwrap();
            peak = peak.max(frames.retained());
            match frame {
                None => continue,
                Some(Frame::Message(message)) => break message,
                Some(Frame::Skipped(_)) => panic!("large snapshot was skipped"),
            }
        };
        // One fixed 64KiB read buffer plus a projection of a few hundred
        // bytes; the old reader allocated the whole 20MiB body.
        assert!(peak <= CHUNK + 4096, "peak {peak}");
        assert!(calls > length / (7 * 1024), "{calls}");
        let params = message.params.unwrap();
        assert_eq!(params.conversation_id.as_deref(), Some(NATIVE));
        let mut thread = protocol::ThreadStatus::default();
        assert!(thread.apply(NATIVE, params.change.unwrap()));
        assert_eq!(thread.status(), expected);
    }
}

#[cfg(unix)]
#[test]
fn live_slow_frame_completes_while_progressing_and_stalled_frame_times_out() {
    use socket::{Frame, Frames};
    use std::{
        io::{self, Write},
        os::unix::net::UnixStream,
        thread,
    };
    let bytes = serde_json::to_vec(&json!({"type":"broadcast","version":11,
        "params":{"conversationId":NATIVE,"hostId":"local","change":snapshot(NATIVE, 0, active(&[]))}}))
    .unwrap();
    let mut frame = (bytes.len() as u32).to_le_bytes().to_vec();
    frame.extend_from_slice(&bytes);

    // Arrives over ~2.8s, longer than the 2s idle limit, in 14 steps.
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    reader
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let pieces: Vec<Vec<u8>> = frame
        .chunks(frame.len().div_ceil(14))
        .map(<[u8]>::to_vec)
        .collect();
    let writing = thread::spawn(move || {
        for piece in pieces {
            writer.write_all(&piece).unwrap();
            thread::sleep(Duration::from_millis(200));
        }
        writer
    });
    let start = Instant::now();
    let mut frames = Frames::default();
    let message = loop {
        match frames.read(&mut reader).unwrap() {
            Some(Frame::Message(message)) => break message,
            Some(Frame::Skipped(_)) => panic!("skipped"),
            None => {}
        }
    };
    assert!(start.elapsed() > Duration::from_secs(2));
    assert_eq!(message.version, Some(11));
    let _writer = writing.join().unwrap();

    // A frame that stops arriving fails after 2s without progress.
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    reader
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    writer.write_all(&frame[..frame.len() / 2]).unwrap();
    let start = Instant::now();
    let mut frames = Frames::default();
    let error = loop {
        match frames.read(&mut reader) {
            Ok(None) => {}
            Ok(Some(_)) => panic!("half a frame completed"),
            Err(error) => break error,
        }
    };
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(start.elapsed() >= Duration::from_secs(2));
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[cfg(unix)]
#[test]
fn live_malformed_and_oversized_frames_skip_only_their_own_body() {
    use socket::{Frame, Frames};
    use std::io::Read;
    let good =
        serde_json::to_vec(&json!({"type":"client-discovery-request","requestId":"next"})).unwrap();
    let cases: Vec<(Vec<u8>, Option<&str>)> = vec![
        (
            format!("{{\"type\":\"broadcast\",\"params\":{{\"conversationId\":\"{NATIVE}\",\"change\":{{\"turns\":[1,,2]}}}}}}")
                .into_bytes(),
            Some(NATIVE),
        ),
        (
            // A kept field larger than the projection bound.
            format!(
                "{{\"type\":\"broadcast\",\"params\":{{\"conversationId\":\"{NATIVE}\",\"status\":\"{}\"}}}}",
                "x".repeat(super::projection::MAX_PROJECTION)
            )
            .into_bytes(),
            Some(NATIVE),
        ),
        // Valid projection, invalid message (null version).
        (
            format!("{{\"type\":\"broadcast\",\"version\":null,\"params\":{{\"conversationId\":\"{NATIVE}\"}}}}")
                .into_bytes(),
            Some(NATIVE),
        ),
        (b"{\"type\":\"broadcast\",\"params\":[".to_vec(), None),
    ];
    for (bad, conversation) in cases {
        let mut bytes = (bad.len() as u32).to_le_bytes().to_vec();
        bytes.extend_from_slice(&bad);
        bytes.extend_from_slice(&(good.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&good);
        let mut stream = bytes.as_slice().chain(std::io::empty());
        let mut frames = Frames::default();
        let mut read = || loop {
            if let Some(frame) = frames.read(&mut stream).unwrap() {
                return frame;
            }
        };
        match read() {
            Frame::Skipped(id) => assert_eq!(id.as_deref(), conversation),
            Frame::Message(_) => panic!("malformed frame parsed"),
        }
        match read() {
            Frame::Message(message) => assert_eq!(message.request_id.as_deref(), Some("next")),
            Frame::Skipped(_) => panic!("framing lost after a skipped body"),
        }
    }
}

#[cfg(unix)]
mod peer {
    use super::*;
    use std::{
        io::{Read, Write},
        os::unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
        thread,
    };

    struct FakePeer {
        root: tempfile::TempDir,
        listener: UnixListener,
    }

    impl FakePeer {
        fn new() -> Self {
            let root = tempfile::TempDir::new_in("/private/tmp").unwrap();
            let ipc = root.path().join(".codex/ipc");
            std::fs::create_dir_all(&ipc).unwrap();
            std::fs::set_permissions(&ipc, std::fs::Permissions::from_mode(0o700)).unwrap();
            Self {
                listener: UnixListener::bind(ipc.join("ipc.sock")).unwrap(),
                root,
            }
        }
        fn service(&self) -> LiveCodexStatus {
            LiveCodexStatus::new(Some(self.root.path()))
        }
        fn accept(&self) -> UnixStream {
            self.listener.set_nonblocking(true).unwrap();
            let started = Instant::now();
            loop {
                match self.listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        return stream;
                    }
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && started.elapsed() < Duration::from_secs(4) =>
                    {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(e) => panic!("fake accept: {e}"),
                }
            }
        }
    }

    fn receive(stream: &mut UnixStream) -> Value {
        let mut header = [0; 4];
        stream.read_exact(&mut header).unwrap();
        let length = u32::from_le_bytes(header) as usize;
        assert!(length <= protocol::MAX_FRAME);
        let mut body = vec![0; length];
        stream.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }
    fn send(stream: &mut UnixStream, value: Value) {
        let bytes = serde_json::to_vec(&value).unwrap();
        stream
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .unwrap();
        stream.write_all(&bytes).unwrap();
    }
    fn respond(stream: &mut UnixStream, request: &Value, result: Value) {
        let mut response = json!({"type":"response","requestId":request["requestId"],"resultType":"success",
            "method":request["method"],"result":result});
        response["handledByClientId"] = json!("owner");
        send(stream, response);
    }
    fn initialize(stream: &mut UnixStream) {
        let request = receive(stream);
        assert_eq!(request["method"], "initialize");
        assert_eq!(request["params"]["clientType"], "xtrace-status-observer");
        assert_eq!(request["version"], 0);
        respond(stream, &request, json!({"clientId":"observer"}));
    }
    fn discover(stream: &mut UnixStream, native: &str) {
        let request = receive(stream);
        assert_eq!(request["method"], "thread-owner-discovery");
        assert_eq!(request["version"], 1);
        assert_eq!(
            request["params"],
            json!({"hostId":"local","conversationId":native})
        );
        respond(stream, &request, json!({}));
        assert_follow(&receive(stream), native, true);
    }
    fn assert_follow(value: &Value, native: &str, follow: bool) {
        assert_eq!(value["method"], "thread-stream-following-changed");
        assert_eq!(value["targetClientIds"], json!(["owner"]));
        assert_eq!(value["version"], 1);
        assert_eq!(
            value["params"],
            json!({"hostId":"local","conversationId":native,"following":follow})
        );
    }
    fn broadcast(change: Value) -> Value {
        json!({"type":"broadcast","method":"thread-stream-state-changed","sourceClientId":"owner","version":11,
            "params":{"hostId":"local","conversationId":NATIVE,"change":change}})
    }
    fn flush(stream: &mut UnixStream) {
        // The response is ordered after earlier updates on this fake stream.
        send(
            stream,
            json!({"type":"client-discovery-request","requestId":"barrier"}),
        );
        assert_eq!(
            receive(stream),
            json!({"type":"client-discovery-response","requestId":"barrier","response":{"canHandle":false}})
        );
    }
    fn wait_status(service: &LiveCodexStatus, view: &str, expected: LiveSessionState) {
        let start = Instant::now();
        loop {
            let result = service.read(&ids(), Some(view), eligible()).unwrap();
            if result.states[0].status == expected {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "wanted {expected:?}, got {:?}",
                result.states
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
    fn ready() -> (FakePeer, LiveCodexStatus, UnixStream, String) {
        let peer = FakePeer::new();
        let service = peer.service();
        let view = register(&service);
        assert_eq!(
            service
                .read(&ids(), Some(&view), eligible())
                .unwrap()
                .states[0]
                .status,
            LiveSessionState::Unknown
        );
        let mut stream = peer.accept();
        initialize(&mut stream);
        discover(&mut stream, NATIVE);
        (peer, service, stream, view)
    }

    #[test]
    fn live_fake_peer_producer_response_shape_and_optional_matching_versions() {
        for include_version in [false, true] {
            let peer = FakePeer::new();
            let service = peer.service();
            let view = register(&service);
            service.read(&ids(), Some(&view), eligible()).unwrap();
            let mut stream = peer.accept();
            for (method, version, result) in [
                ("initialize", 0, json!({"clientId":"observer"})),
                ("thread-owner-discovery", 1, json!({})),
            ] {
                let request = receive(&mut stream);
                assert_eq!(request["method"], method);
                let mut response = json!({"type":"response","requestId":request["requestId"],"method":method,"resultType":"success","handledByClientId":"owner","result":result});
                if include_version {
                    response["version"] = json!(version);
                }
                send(&mut stream, response);
            }
            assert_follow(&receive(&mut stream), NATIVE, true);
            send(&mut stream, broadcast(snapshot(NATIVE, 0, active(&[]))));
            wait_status(&service, &view, LiveSessionState::Running);
            service.shutdown();
        }
    }

    #[test]
    fn live_fake_peer_old_snapshots_stay_unknown_and_current_floor_recovers() {
        let (_peer, service, mut stream, view) = ready();
        send(
            &mut stream,
            broadcast(snapshot(NATIVE, 3, json!({"type":"idle"}))),
        );
        wait_status(&service, &view, LiveSessionState::Idle);
        for _ in 0..2 {
            send(&mut stream, broadcast(snapshot(NATIVE, 2, active(&[]))));
            flush(&mut stream);
            assert_eq!(
                service
                    .read(&ids(), Some(&view), eligible())
                    .unwrap()
                    .states[0]
                    .status,
                LiveSessionState::Unknown
            );
        }
        send(
            &mut stream,
            broadcast(snapshot(NATIVE, 3, json!({"type":"idle"}))),
        );
        wait_status(&service, &view, LiveSessionState::Idle);
        for (pointer, value) in [
            ("/sourceClientId", json!("untrusted-owner")),
            ("/version", json!(10)),
            ("/params/hostId", json!("remote")),
        ] {
            let mut invalid = broadcast(snapshot(NATIVE, 4, active(&[])));
            *invalid.pointer_mut(pointer).unwrap() = value;
            send(&mut stream, invalid);
            send(&mut stream, broadcast(snapshot(NATIVE, 2, active(&[]))));
            flush(&mut stream);
            assert_eq!(
                service
                    .read(&ids(), Some(&view), eligible())
                    .unwrap()
                    .states[0]
                    .status,
                LiveSessionState::Unknown
            );
            send(
                &mut stream,
                broadcast(snapshot(NATIVE, 3, json!({"type":"idle"}))),
            );
            wait_status(&service, &view, LiveSessionState::Idle);
        }
        service.shutdown();
    }

    #[test]
    fn live_fake_peer_same_owner_keeps_floor_but_new_owner_accepts_revision_zero() {
        let (_peer, service, mut stream, view) = ready();
        send(
            &mut stream,
            broadcast(snapshot(NATIVE, 3, json!({"type":"idle"}))),
        );
        wait_status(&service, &view, LiveSessionState::Idle);
        let lost = json!({"type":"broadcast","method":"client-status-changed","params":{"clientId":"owner","connected":false}});
        send(&mut stream, lost.clone());
        wait_status(&service, &view, LiveSessionState::Unknown);
        assert_follow(&receive(&mut stream), NATIVE, false);
        discover(&mut stream, NATIVE);
        for _ in 0..2 {
            send(&mut stream, broadcast(snapshot(NATIVE, 2, active(&[]))));
        }
        flush(&mut stream);
        assert_eq!(
            service
                .read(&ids(), Some(&view), eligible())
                .unwrap()
                .states[0]
                .status,
            LiveSessionState::Unknown
        );
        send(
            &mut stream,
            broadcast(snapshot(NATIVE, 3, json!({"type":"idle"}))),
        );
        wait_status(&service, &view, LiveSessionState::Idle);
        send(&mut stream, lost);
        wait_status(&service, &view, LiveSessionState::Unknown);
        assert_follow(&receive(&mut stream), NATIVE, false);
        let request = receive(&mut stream);
        assert_eq!(request["method"], "thread-owner-discovery");
        send(
            &mut stream,
            json!({"type":"response","requestId":request["requestId"],"resultType":"success","method":"thread-owner-discovery","handledByClientId":"new-owner","result":{}}),
        );
        let follow = receive(&mut stream);
        assert_eq!(follow["targetClientIds"], json!(["new-owner"]));
        let mut fresh = broadcast(snapshot(NATIVE, 0, active(&[])));
        fresh["sourceClientId"] = json!("new-owner");
        send(&mut stream, fresh);
        wait_status(&service, &view, LiveSessionState::Running);
        service.shutdown();
    }

    #[test]
    fn live_fake_peer_connection_reset_requires_initialize_discovery_and_snapshot() {
        let (peer, service, mut stream, view) = ready();
        send(&mut stream, broadcast(snapshot(NATIVE, 3, active(&[]))));
        wait_status(&service, &view, LiveSessionState::Running);
        send(
            &mut stream,
            json!({"type":"broadcast","method":"ipc-connection-reset"}),
        );
        wait_status(&service, &view, LiveSessionState::Unknown);
        assert_follow(&receive(&mut stream), NATIVE, false);
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).unwrap(), 0);
        let mut stream = peer.accept();
        // A state received before initialization/discovery must not restore a claim.
        send(&mut stream, broadcast(snapshot(NATIVE, 4, active(&[]))));
        initialize(&mut stream);
        discover(&mut stream, NATIVE);
        send(&mut stream, broadcast(patches(3, 4, json!([]))));
        flush(&mut stream);
        assert_eq!(
            service
                .read(&ids(), Some(&view), eligible())
                .unwrap()
                .states[0]
                .status,
            LiveSessionState::Unknown
        );
        send(
            &mut stream,
            broadcast(snapshot(NATIVE, 0, json!({"type":"idle"}))),
        );
        wait_status(&service, &view, LiveSessionState::Idle);
        service.shutdown();
    }

    #[test]
    fn live_fake_peer_transitions_silence_shared_views_and_unsubscribe() {
        let (_peer, service, mut stream, view) = ready();
        send(&mut stream, broadcast(snapshot(NATIVE, 0, active(&[]))));
        wait_status(&service, &view, LiveSessionState::Running);
        let second = register(&service);
        service
            .read(
                &[ids()[0].clone(), ids()[0].clone()],
                Some(&second),
                eligible(),
            )
            .unwrap();
        thread::sleep(Duration::from_millis(150));
        wait_status(&service, &view, LiveSessionState::Running); // silence is not idle
        send(
            &mut stream,
            broadcast(patches(
                0,
                1,
                json!([{"op":"add","path":["threadRuntimeStatus","activeFlags",0],"value":"waitingOnApproval"}]),
            )),
        );
        wait_status(&service, &view, LiveSessionState::WaitingApproval);
        service.release(&view).unwrap();
        assert_eq!(
            service
                .read(&ids(), Some(&second), eligible())
                .unwrap()
                .states[0]
                .status,
            LiveSessionState::WaitingApproval
        );
        service.release(&second).unwrap();
        assert_follow(&receive(&mut stream), NATIVE, false);
        assert!(service.read(&ids(), Some(&second), eligible()).is_err());
        assert!(service.read(&ids(), Some(&view), eligible()).is_err());
        service.shutdown();
    }

    #[test]
    fn live_fake_peer_wrong_owner_identity_host_version_and_revision_clear_status() {
        let (_peer, service, mut stream, view) = ready();
        let mut invalid = Vec::new();
        for (pointer, value) in [
            ("/sourceClientId", json!("other-owner")),
            ("/version", json!(10)),
            ("/params/hostId", json!("remote")),
            ("/params/change/conversationState/hostId", json!("remote")),
            ("/params/change/conversationState/id", json!(CHILD)),
        ] {
            let mut value_message = broadcast(snapshot(NATIVE, 1, active(&[])));
            *value_message.pointer_mut(pointer).unwrap() = value;
            invalid.push(value_message);
        }
        invalid.push(broadcast(patches(0, 2, json!([]))));
        let mut missing_version = broadcast(snapshot(NATIVE, 1, active(&[])));
        missing_version.as_object_mut().unwrap().remove("version");
        invalid.push(missing_version);
        for invalid in invalid {
            send(&mut stream, broadcast(snapshot(NATIVE, 1, active(&[]))));
            wait_status(&service, &view, LiveSessionState::Running);
            send(&mut stream, invalid);
            wait_status(&service, &view, LiveSessionState::Unknown);
        }
        send(
            &mut stream,
            broadcast(snapshot(NATIVE, 2, json!({"type":"idle"}))),
        );
        wait_status(&service, &view, LiveSessionState::Idle);
        service.shutdown();
        assert_follow(&receive(&mut stream), NATIVE, false);
    }

    #[test]
    fn live_fake_peer_discovery_decline_owner_loss_disconnect_and_recovery() {
        let (peer, service, mut stream, view) = ready();
        send(
            &mut stream,
            json!({"type":"client-discovery-request","requestId":"ownership-test"}),
        );
        assert_eq!(
            receive(&mut stream),
            json!({"type":"client-discovery-response","requestId":"ownership-test","response":{"canHandle":false}})
        );
        send(&mut stream, broadcast(snapshot(NATIVE, 0, active(&[]))));
        wait_status(&service, &view, LiveSessionState::Running);
        send(
            &mut stream,
            json!({"type":"broadcast","method":"client-status-changed","version":0,
            "params":{"clientId":"owner","status":"disconnected"}}),
        );
        wait_status(&service, &view, LiveSessionState::Unknown);
        assert_follow(&receive(&mut stream), NATIVE, false);
        discover(&mut stream, NATIVE);
        send(&mut stream, broadcast(snapshot(NATIVE, 0, active(&[]))));
        wait_status(&service, &view, LiveSessionState::Running);
        drop(stream);
        wait_status(&service, &view, LiveSessionState::Unknown);
        let mut stream = peer.accept();
        initialize(&mut stream);
        discover(&mut stream, NATIVE);
        send(&mut stream, broadcast(snapshot(NATIVE, 0, active(&[]))));
        wait_status(&service, &view, LiveSessionState::Running);
        service.shutdown();
    }

    #[test]
    fn live_fake_peer_fragmented_large_snapshot_and_oversized_frame() {
        let (_peer, service, mut stream, view) = ready();
        let mut value = broadcast(snapshot(NATIVE, 0, active(&[])));
        value["params"]["change"]["conversationState"]["turns"] = json!("x".repeat(10_200_000));
        let bytes = serde_json::to_vec(&value).unwrap();
        let header = (bytes.len() as u32).to_le_bytes();
        stream.write_all(&header[..2]).unwrap();
        thread::sleep(Duration::from_millis(10));
        stream.write_all(&header[2..]).unwrap();
        // Writing concurrently avoids filling the socket while the worker
        // streams this representative 10.2MB snapshot in bounded chunks.
        let mut writer = stream.try_clone().unwrap();
        let writing = thread::spawn(move || {
            for chunk in bytes.chunks(64 * 1024) {
                writer.write_all(chunk).unwrap();
            }
        });
        wait_status(&service, &view, LiveSessionState::Running);
        writing.join().unwrap();
        stream
            .write_all(&((protocol::MAX_FRAME + 1) as u32).to_le_bytes())
            .unwrap();
        wait_status(&service, &view, LiveSessionState::Unknown);
        service.shutdown();
    }

    #[test]
    fn live_fake_peer_timeout_late_discovery_cannot_subscribe() {
        let peer = FakePeer::new();
        let service = peer.service();
        let view = register(&service);
        service.read(&ids(), Some(&view), eligible()).unwrap();
        let mut stream = peer.accept();
        initialize(&mut stream);
        let first = receive(&mut stream);
        assert_eq!(first["method"], "thread-owner-discovery");
        thread::sleep(Duration::from_millis(2200));
        respond(&mut stream, &first, json!({}));
        send(&mut stream, broadcast(snapshot(NATIVE, 0, active(&[]))));
        thread::sleep(Duration::from_millis(100));
        wait_status(&service, &view, LiveSessionState::Unknown);
        let retry = receive(&mut stream);
        assert_eq!(retry["method"], "thread-owner-discovery");
        assert_ne!(retry["requestId"], first["requestId"]);
        respond(&mut stream, &retry, json!({}));
        assert_follow(&receive(&mut stream), NATIVE, true);
        send(&mut stream, broadcast(snapshot(NATIVE, 0, active(&[]))));
        wait_status(&service, &view, LiveSessionState::Running);
        service.shutdown();
    }

    #[test]
    fn live_fake_peer_navigation_cancels_pending_response_and_leases_unsubscribe() {
        let peer = FakePeer::new();
        let service = peer.service();
        let view = register(&service);
        service.read(&ids(), Some(&view), eligible()).unwrap();
        let mut stream = peer.accept();
        initialize(&mut stream);
        let request = receive(&mut stream);
        service.release(&view).unwrap();
        respond(&mut stream, &request, json!({}));
        thread::sleep(Duration::from_millis(100));
        assert!(service.read(&ids(), Some(&view), eligible()).is_err());
        let mut byte = [0];
        assert_eq!(stream.read(&mut byte).unwrap(), 0);
        let next = register(&service);
        service.read(&ids(), Some(&next), eligible()).unwrap();
        let mut stream = peer.accept();
        initialize(&mut stream);
        discover(&mut stream, NATIVE);
        {
            let mut hub = service.shared.hub.lock().unwrap();
            hub.views.get_mut(&next).unwrap().read_at = Instant::now() - LEASE;
        }
        service.shared.wake.notify_all();
        assert_follow(&receive(&mut stream), NATIVE, false);
        service.shutdown();
    }

    #[test]
    fn live_socket_symlink_and_shared_directory_are_unavailable() {
        let peer = FakePeer::new();
        std::fs::set_permissions(
            peer.root.path().join(".codex/ipc"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let service = peer.service();
        let view = register(&service);
        service.read(&ids(), Some(&view), eligible()).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            service
                .read(&ids(), Some(&view), eligible())
                .unwrap()
                .states[0]
                .status,
            LiveSessionState::Unknown
        );
        peer.listener.set_nonblocking(true).unwrap();
        assert!(peer.listener.accept().is_err());
        service.shutdown();
        let alias = peer.root.path().join("alias");
        std::os::unix::fs::symlink(peer.root.path(), &alias).unwrap();
        let service = LiveCodexStatus::new(Some(&alias));
        let view = register(&service);
        service.read(&ids(), Some(&view), eligible()).unwrap();
        thread::sleep(Duration::from_millis(100));
        assert!(peer.listener.accept().is_err());
        service.shutdown();
    }

    #[test]
    fn live_fake_peer_twenty_mib_snapshot_keeps_connection_and_reads_status() {
        let (_peer, service, mut stream, view) = ready();
        let (length, mut body) =
            super::large_snapshot(NATIVE, 320, &active(&["waitingOnApproval"]));
        assert!(length > protocol::MAX_FRAME / 16 + 4 * 1024 * 1024);
        let mut writer = stream.try_clone().unwrap();
        let writing = thread::spawn(move || {
            writer.write_all(&(length as u32).to_le_bytes()).unwrap();
            let mut chunk = [0; 8 * 1024];
            loop {
                let n = body.read(&mut chunk).unwrap();
                if n == 0 {
                    break;
                }
                writer.write_all(&chunk[..n]).unwrap();
            }
        });
        wait_status(&service, &view, LiveSessionState::WaitingApproval);
        writing.join().unwrap();
        // The same connection keeps answering and applying updates.
        flush(&mut stream);
        send(
            &mut stream,
            broadcast(patches(
                3,
                4,
                json!([{"op":"replace","path":"/threadRuntimeStatus/activeFlags","value":[]}]),
            )),
        );
        wait_status(&service, &view, LiveSessionState::Running);
        service.shutdown();
        assert_follow(&receive(&mut stream), NATIVE, false);
    }

    #[test]
    fn live_fake_peer_attributed_bad_frame_resyncs_and_unattributed_one_reconnects() {
        let (peer, service, mut stream, view) = ready();
        send(&mut stream, broadcast(snapshot(NATIVE, 0, active(&[]))));
        wait_status(&service, &view, LiveSessionState::Running);
        let bad = format!(
            "{{\"type\":\"broadcast\",\"params\":{{\"conversationId\":\"{NATIVE}\",\"change\":{{,}}}}}}"
        );
        stream.write_all(&(bad.len() as u32).to_le_bytes()).unwrap();
        stream.write_all(bad.as_bytes()).unwrap();
        wait_status(&service, &view, LiveSessionState::Unknown);
        // Same connection: it answers, then re-follows for a new snapshot.
        flush(&mut stream);
        assert_follow(&receive(&mut stream), NATIVE, true);
        send(&mut stream, broadcast(snapshot(NATIVE, 1, active(&[]))));
        wait_status(&service, &view, LiveSessionState::Running);

        let bad = b"{\"type\":\"broadcast\",\"params\":[";
        stream.write_all(&(bad.len() as u32).to_le_bytes()).unwrap();
        stream.write_all(bad).unwrap();
        wait_status(&service, &view, LiveSessionState::Unknown);
        assert_follow(&receive(&mut stream), NATIVE, false);
        let mut stream = peer.accept();
        initialize(&mut stream);
        discover(&mut stream, NATIVE);
        send(&mut stream, broadcast(snapshot(NATIVE, 0, active(&[]))));
        wait_status(&service, &view, LiveSessionState::Running);
        service.shutdown();
    }

    /// Reads one real chat's status from the Codex app through the app's
    /// own code path. It sends only what the app sends and unfollows on
    /// shutdown. Run with:
    /// XTRACE_LIVE_CODEX_CHAT=<id> cargo test -p xtrace-desktop --lib \
    ///   live_real_codex_socket -- --ignored --nocapture
    #[test]
    #[ignore = "reads the real ~/.codex/ipc/ipc.sock"]
    fn live_real_codex_socket_reports_status_for_one_chat() {
        let native = std::env::var("XTRACE_LIVE_CODEX_CHAT").expect("XTRACE_LIVE_CODEX_CHAT");
        let home = std::path::PathBuf::from(std::env::var("HOME").unwrap());
        let service = LiveCodexStatus::new(Some(&home));
        let view = register(&service);
        let ids = vec![format!("codex-{native}")];
        let eligible: BTreeSet<String> = [native.clone()].into();
        let start = Instant::now();
        let mut last = None;
        while start.elapsed() < Duration::from_secs(15) {
            let status = service
                .read(&ids, Some(&view), eligible.clone())
                .unwrap()
                .states[0]
                .status;
            if last != Some(status) {
                println!("{:>6}ms {status:?}", start.elapsed().as_millis());
                last = Some(status);
            }
            thread::sleep(Duration::from_millis(50));
        }
        service.shutdown();
        assert_ne!(
            last,
            Some(LiveSessionState::Unknown),
            "no status within 15s"
        );
    }
}
