#![cfg(unix)]
use base64::{Engine, engine::general_purpose::STANDARD};
use rusqlite::Connection;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};
use xt_ingest::native::{
    readers_cli::CancelToken,
    session_compactions::{Counted, Event, Outcome, Reader, Reason, Target, Trigger},
    session_source::IndexedSource,
};
use xt_store::Host;

fn id(n: usize) -> String {
    format!("00000000-0000-4000-8000-{n:012}")
}
fn write(path: &Path, value: &[Value]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path,
        value
            .iter()
            .map(|value| format!("{value}\n"))
            .collect::<String>(),
    )
    .unwrap();
}
fn target(host: Host, native: &str, paths: &[PathBuf]) -> Target {
    Target {
        host,
        native_session_id: native.to_owned(),
        locators: paths
            .iter()
            .map(|path| IndexedSource {
                locator: format!(
                    "{}:{}",
                    if host == Host::Claude {
                        "claude"
                    } else if host == Host::Codex {
                        "codex"
                    } else {
                        "cursor"
                    },
                    path.display()
                ),
                checkpoint: None,
            })
            .collect(),
    }
}
fn boundary(native: &str, n: usize) -> Value {
    json!({"type":"system","subtype":"compact_boundary","uuid":id(n),"sessionId":native,"isSidechain":false})
}
fn claude(home: &Path, native: &str, rows: &[Value]) -> Target {
    let path = home
        .join(".claude/projects/p")
        .join(format!("{native}.jsonl"));
    write(&path, rows);
    target(Host::Claude, native, &[path])
}
fn read(home: &Path, target: &Target) -> Outcome {
    Reader::default()
        .read(home, std::slice::from_ref(target), &CancelToken::new())
        .remove(0)
}

// Generate large fixtures without allocating their bodies in the test either.
fn large_row(path: &Path, prefix: &str, bytes: usize, suffix: &[u8]) {
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(prefix.as_bytes()).unwrap();
    let chunk = [b'x'; 64 * 1024];
    let mut left = bytes;
    while left > 0 {
        let n = left.min(chunk.len());
        file.write_all(&chunk[..n]).unwrap();
        left -= n;
    }
    file.write_all(suffix).unwrap();
}

#[test]
fn claude_large_messages_skip_nested_fake_markers_and_keep_late_ownership() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let t = claude(home.path(), &native, &[boundary(&native, 2)]);
    let path = Path::new(t.locators[0].locator.strip_prefix("claude:").unwrap());
    let suffix = format!(
        r#"","fake":{{"type":"system","subtype":"compact_boundary","uuid":"{}","sessionId":"{}","isSidechain":false}},"nested":[{{"originalSessionId":"foreign"}}]}}],"type":"assistant","sessionId":"{}"}}
"#,
        id(3),
        native,
        native
    );
    // The fake marker is nested in the body, never a record's metadata.
    large_row(
        path,
        "{\"message\":[{\"text\":\"",
        2 * 1024 * 1024,
        suffix.as_bytes(),
    );
    assert_eq!(read(home.path(), &t), Outcome::Count { count: 1 });
    let mut file = fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(
        file,
        "{}",
        json!({"type":"user","sessionId":id(99),"message":"body"})
    )
    .unwrap();
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Ownership));
}

#[test]
fn codex_large_header_message_and_compacted_payload_in_either_field_order() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let path = rollout(home.path(), &native, "", &[]);
    large_row(&path, "{\"payload\":{\"base_instructions\":\"", 2 * 1024 * 1024,
        format!("\",\"id\":\"{native}\",\"history_mode\":\"paginated\"}},\"ordinal\":0,\"type\":\"session_meta\"}}\n").as_bytes());
    large_row(
        &path,
        "{\"payload\":{\"content\":[{\"text\":\"",
        2 * 1024 * 1024,
        b"\\\"type\\\":\\\"compacted\\\"\"}]},\"ordinal\":1,\"type\":\"response_item\"}\n",
    );
    large_row(
        &path,
        "{\"payload\":{\"replacement_history\":[{\"content\":\"",
        2 * 1024 * 1024,
        b"\"}],\"compaction_response_id\":\"own\"},\"ordinal\":2,\"type\":\"compacted\"}\n",
    );
    large_row(
        &path,
        "{\"type\":\"compacted\",\"ordinal\":3,\"payload\":{\"compaction_response_id\":\"own\",\"message\":\"",
        2 * 1024 * 1024,
        b"\"}}\n",
    );
    assert_eq!(
        read(home.path(), &target(Host::Codex, &native, &[path])),
        Outcome::Count { count: 1 }
    );
}

#[test]
fn file_over_256_mib_streams_in_the_existing_deadline() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let path = rollout(home.path(), &native, "", &[ordinary_meta(&native, 0)]);
    large_row(
        &path,
        "{\"payload\":{\"message\":\"",
        257 * 1024 * 1024,
        b"\"},\"ordinal\":1,\"type\":\"response_item\"}\n",
    );
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    writeln!(file, "{}", compact(2, "own")).unwrap();
    assert!(fs::metadata(&path).unwrap().len() > 256 * 1024 * 1024);
    let start = std::time::Instant::now();
    assert_eq!(
        read(home.path(), &target(Host::Codex, &native, &[path])),
        Outcome::Count { count: 1 }
    );
    eprintln!("257 MiB compaction scan: {:?}", start.elapsed());
}

#[test]
fn malformed_large_skipped_bodies_are_unknown() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let path = rollout(home.path(), &native, "", &[]);
    let t = target(Host::Codex, &native, std::slice::from_ref(&path));
    for suffix in [
        b"\\q\"},\"ordinal\":1,\"type\":\"response_item\"}\n".as_slice(),
        b"\xff\"},\"ordinal\":1,\"type\":\"response_item\"}\n",
        b"\\uD800\"},\"ordinal\":1,\"type\":\"response_item\"}\n",
        b"\\uDC00\"},\"ordinal\":1,\"type\":\"response_item\"}\n",
        b"\"],\"ordinal\":1,\"type\":\"response_item\"}\n",
        b"\n",
        b"\"}\n",
    ] {
        write(&path, &[ordinary_meta(&native, 0)]);
        large_row(
            &path,
            "{\"payload\":{\"message\":\"",
            2 * 1024 * 1024,
            suffix,
        );
        assert_eq!(
            read(home.path(), &t),
            Outcome::unknown(Reason::Incomplete),
            "suffix {suffix:?}"
        );
    }
    write(&path, &[ordinary_meta(&native, 0)]);
    large_row(&path, "{\"payload\":{\"message\":\"", 2 * 1024 * 1024, b"");
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Incomplete));
}

#[test]
fn duplicate_metadata_keys_and_deep_ignored_json_are_unknown() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let path = rollout(home.path(), &native, "", &[]);
    let t = target(Host::Codex, &native, std::slice::from_ref(&path));
    for row in [
        r#"{"type":"compacted","type":"response_item","ordinal":1,"payload":{"compaction_response_id":"own"}}"#,
        r#"{"type":"compacted","ordinal":1,"payload":{"compaction_response_id":"own","compaction_response_id":"other"}}"#,
        r#"{"type":"compacted","ordinal":1,"payload":{},"payload":{"compaction_response_id":"own"}}"#,
        r#"{"type":"compacted","ordinal":1,"ord\u0069nal":1,"payload":{"compaction_response_id":"own"}}"#,
    ] {
        write(&path, &[ordinary_meta(&native, 0)]);
        writeln!(
            fs::OpenOptions::new().append(true).open(&path).unwrap(),
            "{row}"
        )
        .unwrap();
        assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Incomplete));
    }
    write(&path, &[ordinary_meta(&native, 0)]);
    writeln!(
        fs::OpenOptions::new().append(true).open(&path).unwrap(),
        "{{\"type\":\"response_item\",\"ordinal\":1,\"payload\":{{\"body\":{}0{}}}}}",
        "[".repeat(129),
        "]".repeat(129)
    )
    .unwrap();
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Limit));
}

#[test]
fn header_projection_preserves_wrong_types_and_spawn_siblings() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let path = rollout(home.path(), &native, "", &[]);
    let t = target(Host::Codex, &native, std::slice::from_ref(&path));
    for (field, value) in [
        ("type", json!(7)),
        ("name", json!({})),
        ("call_id", json!(false)),
        ("history_mode", json!(true)),
        ("history_base", json!([])),
        ("subagent_history_start_ordinal", json!("1")),
    ] {
        let mut header = ordinary_meta(&native, 0);
        header["payload"][field] = value;
        write(&path, &[header, compact(1, "own")]);
        assert_eq!(
            read(home.path(), &t),
            Outcome::unknown(Reason::Ownership),
            "field {field}"
        );
    }
    for source in [
        json!({"subagent":{"thread_spawn":{"parent_thread_id":id(2)},"sibling":null}}),
        json!({"subagent":{"thread_spawn":{"parent_thread_id":id(2)}},"sibling":null}),
    ] {
        let mut header = meta(&native, 0, 1);
        header["payload"]["source"] = source;
        header["payload"]["session_id"] = json!(id(2));
        write(&path, &[header, compact(1, "own")]);
        assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Ownership));
    }
}

#[test]
fn large_rows_preserve_inherited_compactions_ordinals_and_exact_cutoffs() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let root = rollout(home.path(), &native, "", &[meta(&native, 0, 2)]);
    large_row(
        &root,
        "{\"payload\":{\"replacement_history\":[\"",
        2 * 1024 * 1024,
        b"\"],\"compaction_response_id\":\"copied\"},\"type\":\"compacted\",\"ordinal\":1}\n",
    );
    large_row(
        &root,
        "{\"payload\":{\"replacement_history\":[\"",
        2 * 1024 * 1024,
        b"\"],\"compaction_response_id\":\"own\"},\"type\":\"compacted\",\"ordinal\":2}\n",
    );
    let cutoff = fs::metadata(&root).unwrap().len();
    let continuation = rollout(home.path(), &native, &format!("_{}", id(2)), &[]);
    let t = target(Host::Codex, &native, std::slice::from_ref(&root));
    for (bytes, ordinal, outcome) in [
        (cutoff, 3, Outcome::Count { count: 2 }),
        (cutoff - 1, 3, Outcome::unknown(Reason::Incomplete)),
        (cutoff, 4, Outcome::unknown(Reason::Incomplete)),
    ] {
        let mut header = ordinary_meta(&native, ordinal);
        header["payload"]["history_base"] =
            json!({"thread_id":native,"end_byte_offset":bytes,"end_ordinal_exclusive":ordinal});
        write(
            &continuation,
            &[
                header,
                compact(ordinal + 1, "own"),
                compact(ordinal + 2, "second"),
            ],
        );
        assert_eq!(read(home.path(), &t), outcome);
    }
    fs::remove_file(continuation).unwrap();
    writeln!(
        fs::OpenOptions::new().append(true).open(root).unwrap(),
        "{}",
        compact(4, "gap")
    )
    .unwrap();
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Incomplete));
}

#[test]
fn large_claude_checkpoint_rewrites_remain_unknown() {
    use xt_ingest::native::checkpoint::{FileIdentity, TailWindow, file_checkpoint};
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let mut t = claude(home.path(), &native, &[boundary(&native, 2)]);
    let path = PathBuf::from(t.locators[0].locator.strip_prefix("claude:").unwrap());
    large_row(
        &path,
        "{\"type\":\"assistant\",\"message\":\"",
        2 * 1024 * 1024,
        format!("\",\"sessionId\":\"{native}\"}}\n").as_bytes(),
    );
    let bytes = fs::read(&path).unwrap();
    t.locators[0].checkpoint = Some(file_checkpoint(
        &t.locators[0].locator,
        &FileIdentity::of(&fs::metadata(&path).unwrap()),
        bytes.len() as u64,
        &Sha256::new_with_prefix(&bytes),
        &TailWindow::seeded(bytes.clone()),
        2,
        0,
    ));
    assert_eq!(read(home.path(), &t), Outcome::Count { count: 1 });
    use std::io::{Seek, SeekFrom};
    let mut file = fs::OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(1024 * 1024)).unwrap();
    file.write_all(b"y").unwrap();
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Replaced));
}

// Explicitly requested, read-only local controls; never part of default CI.
// Pass the real home and exact root files, with colon-separated paths.
#[test]
#[ignore = "requires explicitly selected local history controls"]
fn real_history_streaming_controls() {
    let home = PathBuf::from(std::env::var_os("XTRACE_COMPACTION_REAL_HOME").unwrap());
    let roots = std::env::var_os("XTRACE_COMPACTION_REAL_ROOTS").unwrap();
    let expected: Vec<u32> = std::env::var("XTRACE_COMPACTION_REAL_COUNTS")
        .unwrap()
        .split(',')
        .map(|n| n.parse().unwrap())
        .collect();
    let paths: Vec<PathBuf> = std::env::split_paths(&roots).collect();
    assert_eq!(paths.len(), expected.len());
    for (path, count) in paths.iter().zip(expected) {
        let stem = path.file_stem().unwrap().to_str().unwrap();
        let native = &stem[stem.len() - 36..];
        let began = std::time::Instant::now();
        let outcome = read(
            &home,
            &target(Host::Codex, native, std::slice::from_ref(path)),
        );
        eprintln!("{native}: {outcome:?} in {:?}", began.elapsed());
        assert_eq!(outcome, Outcome::Count { count });
    }
}

#[test]
fn claude_pairs_duplicates_and_sidechains_are_not_extra_compactions() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let owned = boundary(&native, 2);
    let mut side = boundary(&native, 3);
    side["isSidechain"] = json!(true);
    let t = claude(
        home.path(),
        &native,
        &[
            owned.clone(),
            json!({"type":"user","isCompactSummary":true,"sessionId":native}),
            owned,
            side,
        ],
    );
    assert_eq!(read(home.path(), &t), Outcome::Count { count: 1 });
}
#[test]
fn claude_copied_rewritten_container_with_fork_metadata_is_unknown() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let mut row = boundary(&native, 2);
    row["originalSessionId"] = json!(id(99));
    let t = claude(home.path(), &native, &[row]);
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Ownership));
}
#[test]
fn claude_wrong_owner_and_partial_json_are_unknown_never_zero() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let t = claude(home.path(), &native, &[boundary(&id(99), 2)]);
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Ownership));
    let path = PathBuf::from(t.locators[0].locator.strip_prefix("claude:").unwrap());
    fs::write(&path, "{\"type\":\"user\"}").unwrap();
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Incomplete));
}
#[test]
fn zero_above_five_append_replacement_restart_and_unchanged_cache() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let t = claude(
        home.path(),
        &native,
        &[json!({"type":"user","sessionId":native})],
    );
    let mut reader = Reader::default();
    let token = CancelToken::new();
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::Count { count: 0 }]
    );
    let scans = reader.scans;
    reader.read(home.path(), std::slice::from_ref(&t), &token);
    assert_eq!(reader.scans, scans);
    let path = PathBuf::from(t.locators[0].locator.strip_prefix("claude:").unwrap());
    let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
    for n in 2..9 {
        writeln!(file, "{}", boundary(&native, n)).unwrap();
    }
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::Count { count: 7 }]
    );
    assert_eq!(read(home.path(), &t), Outcome::Count { count: 7 });
    let new = path.with_extension("new");
    write(&new, &[boundary(&native, 10)]);
    fs::rename(new, path).unwrap();
    assert_eq!(
        reader.read(home.path(), &[t], &token),
        vec![Outcome::Count { count: 1 }]
    );
}
#[test]
fn valid_target_ignores_850_unrelated_bad_and_oversized_sources() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let t = claude(home.path(), &native, &[boundary(&native, 2)]);
    for n in 100..950 {
        let path = home
            .path()
            .join(".claude/projects/p")
            .join(format!("{}.jsonl", id(n)));
        fs::write(path, "invalid\n").unwrap();
    }
    let huge = fs::File::create(
        home.path()
            .join(".claude/projects/p")
            .join(format!("{}.jsonl", id(951))),
    )
    .unwrap();
    huge.set_len(3 * 1024 * 1024 * 1024).unwrap();
    let mut reader = Reader::default();
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &CancelToken::new()),
        vec![Outcome::Count { count: 1 }]
    );
    assert_eq!(reader.scans, 1);
    reader.read(home.path(), &[t], &CancelToken::new());
    assert_eq!(reader.scans, 1);
}
#[test]
fn eviction_causes_only_a_future_miss() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let mut reader = Reader::default();
    for n in 1..=270 {
        let native = id(n);
        let t = claude(home.path(), &native, &[boundary(&native, n + 500)]);
        assert_eq!(
            reader.read(home.path(), &[t], &CancelToken::new()),
            vec![Outcome::Count { count: 1 }]
        );
    }
}
fn rollout(home: &Path, native: &str, suffix: &str, rows: &[Value]) -> PathBuf {
    let path = home.join(".codex/sessions/2026/09/30").join(format!(
        "rollout-2026-09-30T00-00-00-{native}{suffix}.jsonl"
    ));
    write(&path, rows);
    path
}
fn meta(native: &str, ordinal: u64, boundary: u64) -> Value {
    json!({"type":"session_meta","ordinal":ordinal,"payload":{"id":native,"history_mode":"paginated","subagent_history_start_ordinal":boundary}})
}
fn compact(ordinal: u64, event: &str) -> Value {
    json!({"type":"compacted","ordinal":ordinal,"payload":{"compaction_response_id":event}})
}
#[test]
fn codex_excludes_inherited_and_deduplicates_own_ids() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let path = rollout(
        home.path(),
        &native,
        "",
        &[
            meta(&native, 0, 2),
            compact(1, "copied"),
            compact(2, "own"),
            compact(3, "own"),
        ],
    );
    assert_eq!(
        read(home.path(), &target(Host::Codex, &native, &[path])),
        Outcome::Count { count: 1 }
    );
}
#[test]
fn codex_one_retained_window_four_is_not_lifetime_four() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let mut marker = compact(1, "retained");
    marker["payload"]["window_number"] = json!(4);
    let path = rollout(home.path(), &native, "", &[meta(&native, 0, 2), marker]);
    assert_eq!(
        read(home.path(), &target(Host::Codex, &native, &[path])),
        Outcome::unknown(Reason::Ownership)
    );
}
#[test]
fn codex_window_number_does_not_prove_dropped_history_or_count() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let mut marker = compact(1, "own");
    marker["payload"]["window_number"] = json!(4);
    let path = rollout(home.path(), &native, "", &[meta(&native, 0, 1), marker]);
    assert_eq!(
        read(home.path(), &target(Host::Codex, &native, &[path])),
        Outcome::Count { count: 1 }
    );
}
#[test]
fn codex_legacy_flat_and_missing_ownership() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let header = json!({"type":"session_meta","payload":{"id":native}});
    let marker = json!({"type":"compacted","payload":{"compaction_response_id":"own"}});
    let path = rollout(home.path(), &native, "", &[header.clone(), marker.clone()]);
    assert_eq!(
        read(
            home.path(),
            &target(Host::Codex, &native, std::slice::from_ref(&path))
        ),
        Outcome::Count { count: 1 }
    );
    let mut inherited = header;
    inherited["payload"]["session_id"] = json!(id(2));
    write(&path, &[inherited, marker]);
    assert_eq!(
        read(home.path(), &target(Host::Codex, &native, &[path])),
        Outcome::unknown(Reason::Ownership)
    );
}
#[test]
fn codex_continuations_prove_named_prefix_and_invalidate_dependency_change() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let rootrows = [meta(&native, 0, 1), compact(1, "shared-event")];
    let root = rollout(home.path(), &native, "", &rootrows);
    let cutoff = fs::metadata(&root).unwrap().len();
    let mut header = meta(&native, 2, 2);
    header["payload"]["history_base"] =
        json!({"thread_id":native,"end_byte_offset":cutoff,"end_ordinal_exclusive":2});
    let continuation = rollout(
        home.path(),
        &native,
        &format!("_{}", id(2)),
        &[header, compact(3, "shared-event"), compact(4, "second")],
    );
    let t = target(Host::Codex, &native, &[root.clone(), continuation]);
    let mut reader = Reader::default();
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &CancelToken::new()),
        vec![Outcome::Count { count: 2 }]
    );
    write(
        &root,
        &[meta(&native, 0, 1), compact(1, "changed-longer-event")],
    );
    assert_eq!(
        reader.read(home.path(), &[t], &CancelToken::new()),
        vec![Outcome::unknown(Reason::Incomplete)]
    );
}
fn hash(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
fn unhex(id: &str) -> Vec<u8> {
    (0..id.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&id[i..i + 2], 16).unwrap())
        .collect()
}
fn node(ids: &[String]) -> Vec<u8> {
    ids.iter()
        .flat_map(|id| [vec![10, 32], unhex(id)].concat())
        .collect()
}
fn leaf(summary: bool) -> Vec<u8> {
    json!({"role":"user","content":"synthetic fixture","providerOptions":{"cursor":{"isSummary":summary}}}).to_string().into_bytes()
}
fn cli(home: &Path, native: &str) -> (PathBuf, Connection) {
    let path = home
        .join(".cursor/chats/project")
        .join(native)
        .join("store.db");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        path.with_file_name("meta.json"),
        json!({"schemaVersion":1}).to_string(),
    )
    .unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE blobs(id TEXT PRIMARY KEY,data BLOB); CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    (path, db)
}
fn blob(db: &Connection, bytes: &[u8]) -> String {
    let id = hash(bytes);
    db.execute(
        "INSERT OR REPLACE INTO blobs VALUES(?1,?2)",
        rusqlite::params![id, bytes],
    )
    .unwrap();
    id
}
fn root(db: &Connection, id: &str, encoded: bool) {
    let v = json!({"latestRootBlobId":id}).to_string();
    let v = if encoded {
        v.as_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    } else {
        v
    };
    db.execute("INSERT OR REPLACE INTO meta VALUES('0',?1)", [v])
        .unwrap();
}
#[test]
fn cursor_reachable_unreachable_hex_meta_and_overlapping_branches() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let (path, db) = cli(home.path(), &native);
    let summary = blob(&db, &leaf(true));
    let ordinary = blob(&db, &leaf(false));
    let unseen = leaf(true);
    let mut unseen: Value = serde_json::from_slice(&unseen).unwrap();
    unseen["content"] = json!("unreachable");
    blob(&db, &serde_json::to_vec(&unseen).unwrap());
    let branch = blob(&db, &node(&[summary.clone(), ordinary]));
    let rootid = blob(&db, &node(&[branch, summary]));
    root(&db, &rootid, true);
    assert_eq!(
        read(home.path(), &target(Host::Cursor, &native, &[path])),
        Outcome::Count { count: 1 }
    );
}
#[test]
fn cursor_schema_missing_reference_and_invalid_cycle_are_unknown() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let (path, db) = cli(home.path(), &native);
    let missing = "a".repeat(64);
    let rootid = blob(&db, &node(std::slice::from_ref(&missing)));
    root(&db, &rootid, false);
    let t = target(Host::Cursor, &native, std::slice::from_ref(&path));
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Incomplete));
    db.execute(
        "INSERT INTO blobs VALUES(?1,?2)",
        rusqlite::params![missing, node(std::slice::from_ref(&missing))],
    )
    .unwrap();
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Incomplete));
    fs::write(path.with_file_name("meta.json"), "{\"schemaVersion\":2}").unwrap();
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Unsupported));
}
#[test]
fn cursor_explicit_shared_copy_is_not_own() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let (path, db) = cli(home.path(), &native);
    let mut summary: Value = serde_json::from_slice(&leaf(true)).unwrap();
    summary["sourceSessionId"] = json!(id(2));
    let summary = blob(&db, &serde_json::to_vec(&summary).unwrap());
    let rootid = blob(&db, &node(&[summary]));
    root(&db, &rootid, false);
    assert_eq!(
        read(home.path(), &target(Host::Cursor, &native, &[path])),
        Outcome::unknown(Reason::Ownership)
    );
}
#[test]
fn cursor_wal_only_changes_read_original_without_creating_shm() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let (path, db) = cli(home.path(), &native);
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    let ordinary = blob(&db, &leaf(false));
    let rootid = blob(&db, &node(&[ordinary]));
    root(&db, &rootid, false);
    let t = target(Host::Cursor, &native, std::slice::from_ref(&path));
    let mut reader = Reader::default();
    let token = CancelToken::new();
    let before = fs::read(&path).unwrap();
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::Count { count: 0 }]
    );
    let summary = blob(&db, &leaf(true));
    let rootid = blob(&db, &node(&[summary]));
    root(&db, &rootid, false);
    assert_eq!(before, fs::read(&path).unwrap());
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::Count { count: 1 }]
    );
    let scans = reader.scans;
    reader.read(home.path(), &[t], &token);
    assert_eq!(reader.scans, scans);
    drop(db);
    fs::remove_file(format!("{}-shm", path.display())).ok();
    fs::remove_file(format!("{}-wal", path.display())).ok();
    let t = target(Host::Cursor, &native, std::slice::from_ref(&path));
    read(home.path(), &t);
    assert!(!Path::new(&format!("{}-shm", path.display())).exists());
}
#[test]
fn ide_exact_roots_ignore_unrelated_malformed_root_and_share_one_snapshot() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let path = home
        .path()
        .join("Library/Application Support/Cursor/User/globalStorage/state.vscdb");
    fs::create_dir_all(home.path().join(".cursor/chats")).unwrap();
    let unrelated = home.path().join("unrelated");
    fs::create_dir_all(&unrelated).unwrap();
    std::os::unix::fs::symlink(&unrelated, home.path().join(".cursor/chats/alias")).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE ItemTable(key TEXT UNIQUE ON CONFLICT REPLACE,value BLOB);CREATE TABLE cursorDiskKV(key TEXT UNIQUE ON CONFLICT REPLACE,value BLOB);").unwrap();
    let summary = leaf(true);
    let hash = hash(&summary);
    db.execute(
        "INSERT INTO cursorDiskKV VALUES(?1,?2)",
        rusqlite::params![format!("agentKv:blob:{hash}"), summary],
    )
    .unwrap();
    for n in 1..=2 {
        let native = id(n);
        let root = STANDARD.encode(node(std::slice::from_ref(&hash)));
        db.execute(
            "INSERT INTO cursorDiskKV VALUES(?1,?2)",
            rusqlite::params![
                format!("composerData:{native}"),
                json!({"composerId":native,"conversationState":format!("~{root}")}).to_string()
            ],
        )
        .unwrap();
        db.execute(
            "INSERT INTO ItemTable VALUES(?1,'invalid decoy root')",
            [format!("composerData:{native}")],
        )
        .unwrap();
    }
    db.execute(
        "INSERT INTO cursorDiskKV VALUES('composerData:unrelated','invalid json')",
        [],
    )
    .unwrap();
    let targets = [
        target(Host::Cursor, &id(1), &[]),
        target(Host::Cursor, &id(2), &[]),
    ];
    let mut reader = Reader::default();
    assert_eq!(
        reader.read(home.path(), &targets, &CancelToken::new()),
        vec![Outcome::Count { count: 1 }; 2]
    );
    let scans = reader.scans;
    reader.read(home.path(), &targets, &CancelToken::new());
    assert_eq!(reader.scans, scans);
    let project = home.path().join(".cursor/chats/project");
    fs::create_dir_all(&project).unwrap();
    std::os::unix::fs::symlink(&unrelated, project.join(id(1))).unwrap();
    fs::write(unrelated.join("store.db"), "unsafe requested store").unwrap();
    assert_eq!(
        reader.read(home.path(), &targets[..1], &CancelToken::new()),
        vec![Outcome::unknown(Reason::IdentityMismatch)]
    );
}

fn refs(field: u8, ids: &[String]) -> Vec<u8> {
    ids.iter()
        .flat_map(|id| [vec![(field << 3) | 2, 32], unhex(id)].concat())
        .collect()
}
fn distinct_summary(n: usize) -> Vec<u8> {
    let mut value: Value = serde_json::from_slice(&leaf(true)).unwrap();
    value["content"] = json!(format!("synthetic summary {n}"));
    serde_json::to_vec(&value).unwrap()
}

#[test]
fn cursor_named_archives_dedupe_current_ignore_unreferenced_and_refresh_wal_cache() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let (path, db) = cli(home.path(), &native);
    db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .unwrap();
    let summary_ids: Vec<_> = (1..=3).map(|n| blob(&db, &distinct_summary(n))).collect();
    let archives: Vec<_> = summary_ids
        .iter()
        .map(|id| blob(&db, &refs(4, std::slice::from_ref(id))))
        .collect();
    // Extra flagged bytes and a field-6 pointer are not selected history.
    let unseen = blob(&db, &distinct_summary(99));
    let current = &summary_ids[2];
    let root_bytes = [
        node(std::slice::from_ref(current)),
        refs(13, &archives),
        refs(6, &[unseen]),
    ]
    .concat();
    let rootid = blob(&db, &root_bytes);
    root(&db, &rootid, true);
    let t = target(Host::Cursor, &native, std::slice::from_ref(&path));
    let token = CancelToken::new();
    let mut reader = Reader::default();
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::Count { count: 3 }]
    );
    reader.read(home.path(), std::slice::from_ref(&t), &token);
    assert_eq!(reader.scans, 1);
    // An archive generation changes while main DB bytes remain unchanged.
    let main_before = fs::read(&path).unwrap();
    let only_current = blob(
        &db,
        &[
            node(std::slice::from_ref(current)),
            refs(13, &archives[2..]),
        ]
        .concat(),
    );
    root(&db, &only_current, true);
    assert_eq!(fs::read(&path).unwrap(), main_before);
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::Count { count: 1 }]
    );
    assert_eq!(reader.scans, 2);
    let cancelled = CancelToken::new();
    cancelled.cancel();
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &cancelled),
        vec![Outcome::unknown(Reason::Cancelled)]
    );
    drop(db);
    let replacement = path.with_file_name("replacement.db");
    let new = Connection::open(&replacement).unwrap();
    new.execute_batch("CREATE TABLE blobs(id TEXT PRIMARY KEY,data BLOB); CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT);").unwrap();
    let ordinary = blob(&new, &leaf(false));
    let fresh = blob(&new, &node(&[ordinary]));
    root(&new, &fresh, false);
    drop(new);
    fs::rename(replacement, &path).unwrap();
    assert_eq!(
        reader.read(home.path(), &[t], &token),
        vec![Outcome::Count { count: 0 }]
    );
}

#[test]
fn cursor_archives_require_exact_typed_summary_pointer_and_valid_owned_marker() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let (path, db) = cli(home.path(), &native);
    let t = target(Host::Cursor, &native, &[path]);
    let own = blob(&db, &distinct_summary(1));
    let ordinary = blob(&db, &leaf(false));
    let mut foreign: Value = serde_json::from_slice(&distinct_summary(2)).unwrap();
    foreign["sourceSessionId"] = json!(id(2));
    let foreign = blob(&db, &serde_json::to_vec(&foreign).unwrap());
    let missing = "a".repeat(64);
    for archive in [
        missing.clone(),
        blob(&db, &refs(4, std::slice::from_ref(&missing))),
        blob(&db, &node(std::slice::from_ref(&own))), // field 1 is not summary_message
        blob(&db, &refs(4, &[own.clone(), own.clone()])), // ambiguous singular field
        blob(&db, &refs(4, std::slice::from_ref(&ordinary))),
        blob(&db, &refs(4, &[foreign])),
        own.clone(), // JSON cannot be used as an archive protobuf
    ] {
        // Current ordinary/summary visits cannot bypass archive validation.
        let rootid = blob(
            &db,
            &[node(&[own.clone(), ordinary.clone()]), refs(13, &[archive])].concat(),
        );
        root(&db, &rootid, false);
        assert!(matches!(read(home.path(), &t), Outcome::Unknown { .. }));
    }
    let bad_hash = "b".repeat(64);
    db.execute(
        "INSERT INTO blobs VALUES(?1,?2)",
        rusqlite::params![bad_hash, refs(4, std::slice::from_ref(&own))],
    )
    .unwrap();
    let rootid = blob(&db, &refs(13, &[bad_hash]));
    root(&db, &rootid, false);
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Incomplete));
    let truncated_pointer = blob(&db, &[106, 31, 0]);
    root(&db, &truncated_pointer, false);
    assert!(matches!(read(home.path(), &t), Outcome::Unknown { .. }));
}

#[test]
fn cursor_archive_fields_on_message_nodes_are_not_followed() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let (path, db) = cli(home.path(), &native);
    let ordinary = blob(&db, &leaf(false));
    let messages = blob(
        &db,
        &[node(&[ordinary]), refs(13, &["a".repeat(64)])].concat(),
    );
    let rootid = blob(&db, &node(&[messages]));
    root(&db, &rootid, false);
    assert_eq!(
        read(home.path(), &target(Host::Cursor, &native, &[path])),
        Outcome::Count { count: 0 }
    );
}

#[test]
fn cursor_populated_legacy_root_archive_is_unknown_not_current_only_or_zero() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let (path, db) = cli(home.path(), &native);
    let t = target(Host::Cursor, &native, &[path]);
    let summary = blob(&db, &distinct_summary(1));
    let ordinary = blob(&db, &leaf(false));
    let archive = blob(&db, &refs(4, std::slice::from_ref(&summary)));
    for (current, expected) in [(summary.clone(), 1), (ordinary, 0)] {
        for suffix in [Vec::new(), vec![90, 0]] {
            let rootid = blob(
                &db,
                &[node(std::slice::from_ref(&current)), suffix].concat(),
            );
            root(&db, &rootid, false);
            assert_eq!(read(home.path(), &t), Outcome::Count { count: expected });
        }
        // Deliberately no blob at the pointer: the guard must refuse the
        // legacy format without traversing it or returning the current count.
        let rootid = blob(
            &db,
            &[
                node(std::slice::from_ref(&current)),
                refs(11, &["a".repeat(64)]),
            ]
            .concat(),
        );
        root(&db, &rootid, false);
        assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Unsupported));
    }
    let legacy = refs(11, &["a".repeat(64)]);
    for prefix in [
        Vec::new(),
        refs(13, std::slice::from_ref(&archive)),
        [
            node(std::slice::from_ref(&summary)),
            refs(13, std::slice::from_ref(&archive)),
        ]
        .concat(),
    ] {
        let rootid = blob(&db, &[prefix, legacy.clone()].concat());
        root(&db, &rootid, false);
        assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Unsupported));
    }
    for suffix in [Vec::new(), vec![90, 0]] {
        let rootid = blob(
            &db,
            &[refs(13, std::slice::from_ref(&archive)), suffix].concat(),
        );
        root(&db, &rootid, false);
        assert_eq!(read(home.path(), &t), Outcome::Count { count: 1 });
    }
}

#[test]
fn ide_actual_table_accepts_text_and_blob_roots_and_bounds_bytes_before_copy() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let path = home
        .path()
        .join("Library/Application Support/Cursor/User/globalStorage/state.vscdb");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let db = Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE ItemTable(key TEXT UNIQUE ON CONFLICT REPLACE,value BLOB); CREATE TABLE cursorDiskKV(key TEXT UNIQUE ON CONFLICT REPLACE,value BLOB);").unwrap();
    let summary = distinct_summary(1);
    let hash = hash(&summary);
    db.execute(
        "INSERT INTO cursorDiskKV VALUES(?1,?2)",
        rusqlite::params![format!("agentKv:blob:{hash}"), summary],
    )
    .unwrap();
    let key = format!("composerData:{native}");
    let root = json!({"composerId":native,"conversationState":format!("~{}", STANDARD.encode(node(&[hash])))}).to_string();
    db.execute("INSERT INTO ItemTable VALUES(?1,'malformed decoy')", [&key])
        .unwrap();
    db.execute(
        "INSERT INTO cursorDiskKV VALUES('composerData:unrelated','malformed root')",
        [],
    )
    .unwrap();
    let t = target(Host::Cursor, &native, &[]);
    for value in [
        rusqlite::types::Value::Text(root.clone()),
        rusqlite::types::Value::Blob(root.as_bytes().to_vec()),
    ] {
        db.execute(
            "INSERT OR REPLACE INTO cursorDiskKV VALUES(?1,?2)",
            rusqlite::params![key, value],
        )
        .unwrap();
        assert_eq!(read(home.path(), &t), Outcome::Count { count: 1 });
    }
    // Fewer than 1 Mi characters, but more than 1 Mi UTF-8 bytes.
    let large = json!({"composerId":native,"padding":"é".repeat(600_000),"conversationState":"~"})
        .to_string();
    assert!(large.chars().count() < 1024 * 1024 && large.len() > 1024 * 1024);
    for value in [
        rusqlite::types::Value::Text(large.clone()),
        rusqlite::types::Value::Blob(large.into_bytes()),
    ] {
        db.execute(
            "INSERT OR REPLACE INTO cursorDiskKV VALUES(?1,?2)",
            rusqlite::params![key, value],
        )
        .unwrap();
        assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Limit));
    }
    for value in [
        rusqlite::types::Value::Integer(7),
        rusqlite::types::Value::Null,
        rusqlite::types::Value::Blob(vec![255]),
    ] {
        db.execute(
            "INSERT OR REPLACE INTO cursorDiskKV VALUES(?1,?2)",
            rusqlite::params![key, value],
        )
        .unwrap();
        assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Unsupported));
    }
    db.execute("DELETE FROM cursorDiskKV WHERE key=?1", [&key])
        .unwrap();
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Missing));
}
#[test]
fn aliases_missing_cancel_and_bad_identifier_never_fake_zero() {
    let home = tempfile::Builder::new()
        .prefix("xtrace-compaction-test-")
        .tempdir_in("/private/tmp")
        .unwrap();
    let native = id(1);
    let t = claude(home.path(), &native, &[boundary(&native, 2)]);
    let token = CancelToken::new();
    token.cancel();
    assert_eq!(
        Reader::default().read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::unknown(Reason::Cancelled)]
    );
    let path = PathBuf::from(t.locators[0].locator.strip_prefix("claude:").unwrap());
    fs::remove_file(&path).unwrap();
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Missing));
    let other = home.path().join("outside.jsonl");
    write(&other, &[boundary(&native, 2)]);
    std::os::unix::fs::symlink(other, &path).unwrap();
    assert!(matches!(read(home.path(), &t), Outcome::Unknown { .. }));
}

#[test]
fn claude_ordinary_rows_need_owner_proof_but_progress_does_not() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    for row in [
        json!({"type":"user"}),
        json!({"type":"user","sessionId":id(2)}),
    ] {
        let t = claude(home.path(), &native, &[row]);
        assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Ownership));
    }
    let t = claude(
        home.path(),
        &native,
        &[
            json!({"type":"user","sessionId":native}),
            json!({"type":"progress"}),
            json!({"type":"custom-title","customTitle":"fixture"}),
            json!({"type":"user","sessionId":native,"isCompactSummary":true}),
        ],
    );
    assert_eq!(read(home.path(), &t), Outcome::Count { count: 0 });
}

fn ordinary_meta(native: &str, ordinal: u64) -> Value {
    json!({"type":"session_meta","ordinal":ordinal,"payload":{"id":native,"history_mode":"paginated"}})
}

#[test]
fn codex_root_only_locator_discovers_new_continuations_and_abandoned_tail() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let rootrows = [ordinary_meta(&native, 0), compact(1, "first")];
    let rootpath = rollout(home.path(), &native, "", &rootrows);
    let cutoff = fs::metadata(&rootpath).unwrap().len();
    let t = target(Host::Codex, &native, std::slice::from_ref(&rootpath));
    let mut reader = Reader::default();
    let token = CancelToken::new();
    for n in 100..950 {
        rollout(
            home.path(),
            &id(n),
            "",
            &[json!({"irrelevant":"malformed header"})],
        );
    }
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::Count { count: 1 }]
    );
    reader.read(home.path(), std::slice::from_ref(&t), &token);
    assert_eq!(reader.scans, 1);
    let mut header = ordinary_meta(&native, 2);
    header["payload"]["history_base"] =
        json!({"thread_id":native,"end_byte_offset":cutoff,"end_ordinal_exclusive":2});
    let continuation = rollout(
        home.path(),
        &native,
        &format!("_{}", id(2)),
        &[header.clone(), compact(3, "second")],
    );
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::Count { count: 2 }]
    );
    assert_eq!(reader.scans, 2);
    fs::OpenOptions::new()
        .append(true)
        .open(&rootpath)
        .unwrap()
        .write_all(format!("{}\n", compact(2, "abandoned")).as_bytes())
        .unwrap();
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::Count { count: 3 }]
    );
    fs::remove_file(continuation).unwrap();
    assert_eq!(
        reader.read(home.path(), std::slice::from_ref(&t), &token),
        vec![Outcome::Count { count: 2 }]
    );
    let parked = rootpath.with_extension("parked");
    fs::rename(&rootpath, &parked).unwrap();
    std::os::unix::fs::symlink(&parked, &rootpath).unwrap();
    assert!(matches!(
        reader.read(home.path(), &[t], &token)[0],
        Outcome::Unknown { .. }
    ));
}

#[test]
fn codex_boundary_does_not_authorize_foreign_or_forked_history() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let mut header = meta(&native, 0, 2);
    header["payload"]["session_id"] = json!(id(2));
    let path = rollout(
        home.path(),
        &native,
        "",
        &[header.clone(), compact(1, "inherited"), compact(2, "own")],
    );
    let t = target(Host::Codex, &native, std::slice::from_ref(&path));
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Ownership));
    header["payload"]["source"] = json!({"subagent":{"thread_spawn":{"parent_thread_id":id(2)}}});
    write(
        &path,
        &[header.clone(), compact(1, "inherited"), compact(2, "own")],
    );
    assert_eq!(read(home.path(), &t), Outcome::Count { count: 1 });
    header["payload"]["session_id"] = json!(id(3));
    write(
        &path,
        &[header.clone(), compact(1, "inherited"), compact(2, "own")],
    );
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Ownership));
    header["payload"]["session_id"] = json!(id(2));
    header["payload"]["forked_from_id"] = json!(id(4));
    write(&path, &[header, compact(1, "inherited"), compact(2, "own")]);
    assert_eq!(read(home.path(), &t), Outcome::unknown(Reason::Ownership));
}

#[test]
fn cursor_walk_matches_owner_and_refuses_foreign_and_unresolved_fork() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let (path, db) = cli(home.path(), &native);
    let t = target(Host::Cursor, &native, &[path]);
    for (field, value, expected) in [
        (
            "sourceSessionId",
            json!(native),
            Outcome::Count { count: 1 },
        ),
        (
            "originalSessionId",
            json!(native),
            Outcome::Count { count: 1 },
        ),
        (
            "sourceSessionId",
            json!(id(2)),
            Outcome::unknown(Reason::Ownership),
        ),
        (
            "parentComposerId",
            json!(native),
            Outcome::unknown(Reason::Ownership),
        ),
        ("isFork", json!(true), Outcome::unknown(Reason::Ownership)),
    ] {
        let mut summary: Value = serde_json::from_slice(&leaf(true)).unwrap();
        summary[field] = value;
        let hash = blob(&db, &serde_json::to_vec(&summary).unwrap());
        let rootid = blob(&db, &node(&[hash]));
        root(&db, &rootid, false);
        assert_eq!(read(home.path(), &t), expected, "{field}");
    }
}

// Compaction times and triggers ride on the same read as the count.
fn read_events(home: &Path, target: &Target) -> Counted {
    Reader::default()
        .read_events(home, std::slice::from_ref(target), &CancelToken::new())
        .remove(0)
}
fn at(text: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(text)
        .unwrap()
        .timestamp_millis()
}
fn timed(mut row: Value, timestamp: &str) -> Value {
    row["timestamp"] = json!(timestamp);
    row
}
fn triggered(native: &str, n: usize, timestamp: &str, trigger: &str) -> Value {
    let mut row = timed(boundary(native, n), timestamp);
    // Large preserved context beside the trigger is never kept.
    row["compactMetadata"] = json!({
        "trigger": trigger,
        "preTokens": 1,
        "preservedMessages": vec!["x".repeat(1024); 128],
    });
    row
}

#[test]
fn claude_events_carry_time_and_trigger_one_per_counted_marker() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let auto = triggered(&native, 2, "2026-09-07T12:00:00.250Z", "auto");
    let mut side = triggered(&native, 5, "2026-09-07T12:30:00Z", "manual");
    side["isSidechain"] = json!(true);
    let t = claude(
        home.path(),
        &native,
        &[
            triggered(&native, 3, "2026-09-07T13:00:00+02:00", "manual"),
            auto.clone(),
            json!({"type":"user","isCompactSummary":true,"sessionId":native}),
            // A repeated marker is one compaction, at its first recorded time.
            timed(auto, "2026-09-07T14:00:00Z"),
            side,
            timed(boundary(&native, 4), "2026-09-07T13:30:00Z"),
        ],
    );
    let counted = read_events(home.path(), &t);
    assert_eq!(counted.outcome, Outcome::Count { count: 3 });
    assert_eq!(read(home.path(), &t), counted.outcome);
    assert_eq!(
        counted.events,
        vec![
            Event {
                at_ms: at("2026-09-07T11:00:00Z"),
                trigger: Trigger::Manual
            },
            Event {
                at_ms: at("2026-09-07T12:00:00.250Z"),
                trigger: Trigger::Auto
            },
            // No compactMetadata: counted, trigger not recorded.
            Event {
                at_ms: at("2026-09-07T13:30:00Z"),
                trigger: Trigger::Unknown
            },
        ]
    );
}

#[test]
fn claude_marker_without_a_readable_time_is_counted_with_no_event() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let t = claude(
        home.path(),
        &native,
        &[
            triggered(&native, 2, "2026-09-07T12:00:00Z", "auto"),
            boundary(&native, 3),
            timed(boundary(&native, 4), "yesterday"),
        ],
    );
    let counted = read_events(home.path(), &t);
    assert_eq!(counted.outcome, Outcome::Count { count: 3 });
    assert_eq!(
        counted.events,
        vec![Event {
            at_ms: at("2026-09-07T12:00:00Z"),
            trigger: Trigger::Auto
        }]
    );
}

#[test]
fn claude_unknown_count_has_no_events() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let t = claude(
        home.path(),
        &native,
        &[
            triggered(&native, 2, "2026-09-07T12:00:00Z", "auto"),
            json!({"type":"user","sessionId":id(99)}),
        ],
    );
    assert_eq!(
        read_events(home.path(), &t),
        Counted {
            outcome: Outcome::unknown(Reason::Ownership),
            events: Vec::new()
        }
    );
}

#[test]
fn codex_events_have_time_unknown_trigger_and_skip_inherited_rows() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let path = rollout(
        home.path(),
        &native,
        "",
        &[
            meta(&native, 0, 2),
            timed(compact(1, "copied"), "2026-09-01T00:00:00Z"),
            timed(compact(2, "own"), "2026-09-07T10:00:00.5Z"),
            timed(compact(3, "own"), "2026-09-07T11:00:00Z"),
            timed(compact(4, "second"), "2026-09-07T09:00:00Z"),
        ],
    );
    let counted = read_events(home.path(), &target(Host::Codex, &native, &[path]));
    assert_eq!(counted.outcome, Outcome::Count { count: 2 });
    assert_eq!(
        counted.events,
        vec![
            Event {
                at_ms: at("2026-09-07T09:00:00Z"),
                trigger: Trigger::Unknown
            },
            Event {
                at_ms: at("2026-09-07T10:00:00.5Z"),
                trigger: Trigger::Unknown
            },
        ]
    );
}

#[test]
fn codex_continuation_events_match_the_count_and_refresh_with_it() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let rootrows = [
        meta(&native, 0, 1),
        timed(compact(1, "shared-event"), "2026-09-07T08:00:00Z"),
    ];
    let root = rollout(home.path(), &native, "", &rootrows);
    let cutoff = fs::metadata(&root).unwrap().len();
    let mut header = meta(&native, 2, 2);
    header["payload"]["history_base"] =
        json!({"thread_id":native,"end_byte_offset":cutoff,"end_ordinal_exclusive":2});
    let continuation = rollout(
        home.path(),
        &native,
        &format!("_{}", id(2)),
        &[
            header.clone(),
            timed(compact(3, "shared-event"), "2026-09-07T08:30:00Z"),
            timed(compact(4, "second"), "2026-09-07T09:00:00Z"),
        ],
    );
    let t = target(Host::Codex, &native, &[root, continuation.clone()]);
    let mut reader = Reader::default();
    let first = reader.read_events(home.path(), std::slice::from_ref(&t), &CancelToken::new());
    assert_eq!(first[0].outcome, Outcome::Count { count: 2 });
    assert_eq!(
        first[0].events.iter().map(|e| e.at_ms).collect::<Vec<_>>(),
        vec![at("2026-09-07T08:00:00Z"), at("2026-09-07T09:00:00Z")]
    );
    // A cached answer keeps its events; a changed source reads them again.
    assert_eq!(
        reader.read_events(home.path(), std::slice::from_ref(&t), &CancelToken::new()),
        first
    );
    let scans = reader.scans;
    write(
        &continuation,
        &[
            header,
            timed(compact(3, "shared-event"), "2026-09-07T08:30:00Z"),
            timed(compact(4, "second"), "2026-09-07T09:00:00Z"),
            timed(compact(5, "third"), "2026-09-07T10:00:00Z"),
        ],
    );
    let second = reader.read_events(home.path(), &[t], &CancelToken::new());
    assert_eq!(reader.scans, scans + 1);
    assert_eq!(second[0].outcome, Outcome::Count { count: 3 });
    assert_eq!(second[0].events.len(), 3);
}

#[test]
fn cursor_counts_have_no_events() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let (path, db) = cli(home.path(), &native);
    let summary = blob(&db, &leaf(true));
    let rootid = blob(&db, &node(&[summary]));
    root(&db, &rootid, false);
    assert_eq!(
        read_events(home.path(), &target(Host::Cursor, &native, &[path])),
        Counted {
            outcome: Outcome::Count { count: 1 },
            events: Vec::new()
        }
    );
}

// Odd times and triggers are display-only: each row counts exactly as it
// did before they were read, with no time or an unknown trigger.
#[test]
fn odd_compaction_metadata_and_times_never_change_a_count() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let at_noon = "2026-09-07T12:00:00Z";
    let mut rows = Vec::new();
    // compactMetadata over 64 KiB as a string, as a large array, and as an
    // object holding an oversized trigger and a key over 256 bytes.
    for (n, metadata) in [
        (2, json!("x".repeat(80 * 1024))),
        (3, json!(vec!["x".repeat(1024); 96])),
        (4, json!({"trigger": "x".repeat(80 * 1024)})),
        (5, json!({ "k".repeat(300): 1, "trigger": "auto" })),
        (6, json!({"trigger": ["auto"]})),
        (7, json!(null)),
    ] {
        let mut row = timed(boundary(&native, n), at_noon);
        row["compactMetadata"] = metadata;
        rows.push(row.to_string());
    }
    // Times that are not strings, or are oversized strings.
    for (n, timestamp) in [
        (8, json!(1_788_782_400_000_i64)),
        (9, json!({"at": at_noon})),
        (10, json!("9".repeat(80 * 1024))),
    ] {
        let mut row = boundary(&native, n);
        row["timestamp"] = timestamp;
        rows.push(row.to_string());
    }
    // A duplicate key inside compactMetadata, which only JSON text can hold.
    let mut row = timed(boundary(&native, 11), at_noon);
    row["compactMetadata"] = json!({"trigger": "auto"});
    rows.push(row.to_string().replace(
        r#""compactMetadata":{"trigger":"auto"}"#,
        r#""compactMetadata":{"trigger":"auto","trigger":"manual"}"#,
    ));
    assert!(rows.last().unwrap().contains(r#""trigger":"manual""#));
    let path = home
        .path()
        .join(".claude/projects/p")
        .join(format!("{native}.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, rows.join("\n") + "\n").unwrap();
    let t = target(Host::Claude, &native, &[path]);
    let counted = read_events(home.path(), &t);
    assert_eq!(counted.outcome, Outcome::Count { count: 10 });
    assert_eq!(read(home.path(), &t), counted.outcome);
    // Rows 2–7 and 11 keep their time; 8–10 have none. Only row 5's plain
    // trigger is read: a long sibling key is not "trigger". Every other
    // trigger is not a single short string, so it is not recorded.
    let mut expected = vec![
        Event {
            at_ms: at(at_noon),
            trigger: Trigger::Unknown
        };
        7
    ];
    expected[0].trigger = Trigger::Auto;
    assert_eq!(counted.events, expected);

    // Codex: a time that is not a string is skipped; the count is unchanged.
    let path = rollout(
        home.path(),
        &native,
        "",
        &[
            ordinary_meta(&native, 0),
            json!({"type":"compacted","ordinal":1,"timestamp":12,"payload":{"compaction_response_id":"a"}}),
            json!({"type":"compacted","ordinal":2,"timestamp":["x"],"payload":{"compaction_response_id":"b"}}),
            json!({"type":"compacted","ordinal":3,"timestamp":"x".repeat(80 * 1024),"payload":{"compaction_response_id":"c"}}),
        ],
    );
    assert_eq!(
        read_events(home.path(), &target(Host::Codex, &native, &[path])),
        Counted {
            outcome: Outcome::Count { count: 3 },
            events: Vec::new()
        }
    );
}

// The reviewer's repro: a time read early never takes projection room from a
// later field that still fits on its own.
#[test]
fn an_early_time_never_crowds_out_a_large_projected_field() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let path = home
        .path()
        .join(".claude/projects/p")
        .join(format!("{native}.jsonl"));
    let wide = format!(
        r#"{{"timestamp":"2026-09-07T12:00:00Z","type":"progress","sessionId":"{native}","uuid":"{}"}}"#,
        "u".repeat(65_440)
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(
        &path,
        format!(
            "{wide}\n{}\n",
            timed(boundary(&native, 2), "2026-09-07T12:00:01Z")
        ),
    )
    .unwrap();
    let t = target(Host::Claude, &native, &[path]);
    assert_eq!(read(home.path(), &t), Outcome::Count { count: 1 });
    assert_eq!(
        read_events(home.path(), &t).events,
        vec![Event {
            at_ms: at("2026-09-07T12:00:01Z"),
            trigger: Trigger::Unknown
        }]
    );
}

// The reviewer's repro: millions of compactMetadata keys are skipped as
// before, never collected, and a trigger after them is still read.
#[test]
fn millions_of_compact_metadata_keys_are_streamed_not_collected() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let path = home
        .path()
        .join(".claude/projects/p")
        .join(format!("{native}.jsonl"));
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut file = fs::File::create(&path).unwrap();
    let head = timed(boundary(&native, 2), "2026-09-07T12:00:00Z").to_string();
    // {...,"compactMetadata":{"k0":0,...,"trigger":"manual"}}
    write!(file, "{},\"compactMetadata\":{{", &head[..head.len() - 1]).unwrap();
    let mut keys = String::new();
    for n in 0..3_000_000 {
        keys.push_str(&format!("\"k{n}\":0,"));
        if keys.len() > 1 << 20 {
            file.write_all(keys.as_bytes()).unwrap();
            keys.clear();
        }
    }
    writeln!(file, "{keys}\"trigger\":\"manual\"}}}}").unwrap();
    drop(file);
    assert!(fs::metadata(&path).unwrap().len() > 30 * 1024 * 1024);
    let t = target(Host::Claude, &native, &[path]);
    assert_eq!(read(home.path(), &t), Outcome::Count { count: 1 });
    assert_eq!(
        read_events(home.path(), &t).events,
        vec![Event {
            at_ms: at("2026-09-07T12:00:00Z"),
            trigger: Trigger::Manual
        }]
    );
}

#[test]
fn a_trigger_after_large_preserved_messages_is_read_and_odd_metadata_is_unknown() {
    let home = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
    let native = id(1);
    let mut late = timed(boundary(&native, 2), "2026-09-07T12:00:00Z");
    late["compactMetadata"] = json!({
        "preservedMessages": vec!["x".repeat(1024); 128],
        "nested": {"trigger": "auto"},
        "trigger": "manual",
    });
    let mut odd = timed(boundary(&native, 3), "2026-09-07T13:00:00Z");
    odd["compactMetadata"] = json!(["trigger", "auto"]);
    let mut escaped = timed(boundary(&native, 4), "2026-09-07T14:00:00Z");
    escaped["compactMetadata"] = json!({"trigger": "au\"to"});
    let t = claude(home.path(), &native, &[late, odd, escaped]);
    let counted = read_events(home.path(), &t);
    assert_eq!(counted.outcome, Outcome::Count { count: 3 });
    assert_eq!(
        counted.events.iter().map(|e| e.trigger).collect::<Vec<_>>(),
        vec![Trigger::Manual, Trigger::Unknown, Trigger::Unknown]
    );
}
