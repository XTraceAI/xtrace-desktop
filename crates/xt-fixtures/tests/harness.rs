use rusqlite::Connection;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;
use xt_fixtures::{Error, Fixture, FixtureId, FixtureStatus, LoadedSession, RULE_IDS, RuleId};
use xt_store::Store;

fn catalog() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
}
fn f1() -> Fixture {
    Fixture::load(catalog().join("F1")).unwrap()
}
fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &destination.join(entry.file_name()));
        } else {
            fs::copy(entry.path(), destination.join(entry.file_name())).unwrap();
        }
    }
}
fn editable(id: &str) -> TempDir {
    let directory = TempDir::new().unwrap();
    copy_tree(&catalog().join(id), directory.path());
    directory
}
fn edit_json(path: &Path, change: impl FnOnce(&mut Value)) {
    let mut value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    change(&mut value);
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}
fn edit_record(directory: &Path, index: usize, change: impl FnOnce(&mut Value)) {
    let path = directory.join("input/sessions/session.jsonl");
    let mut rows = fs::read_to_string(&path)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect::<Vec<Value>>();
    change(&mut rows[index]);
    fs::write(
        path,
        rows.iter()
            .map(|r| serde_json::to_string(r).unwrap() + "\n")
            .collect::<String>(),
    )
    .unwrap();
}
fn load_error(path: &Path) -> String {
    match Fixture::load(path) {
        Ok(_) => panic!("malformed fixture loaded"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn complete_catalog_distinguishes_populated_from_unimplemented() {
    let fixtures = Fixture::all(catalog()).unwrap();
    assert_eq!(fixtures.len(), 21);
    assert_eq!(
        fixtures.iter().map(|f| f.manifest().id).collect::<Vec<_>>(),
        FixtureId::all().collect::<Vec<_>>()
    );
    let mut populated = 0;
    for fixture in fixtures {
        assert_eq!(fixture.now().to_rfc3339(), "2026-09-08T00:00:00+00:00");
        assert_eq!(
            fixture.window_start().to_rfc3339(),
            "2026-09-01T00:00:00+00:00"
        );
        if fixture.manifest().status == FixtureStatus::Populated {
            populated += 1;
            fixture.assert_reference().unwrap();
        } else {
            assert!(matches!(
                fixture.assert_reference(),
                Err(Error::Unimplemented(_))
            ));
            assert!(matches!(
                fixture.assert_expectation(fixture.manifest().proves[0].as_str(), &Value::Null),
                Err(Error::Unimplemented(_))
            ));
            assert!(matches!(
                fixture.build_db(true),
                Err(Error::Unimplemented(_))
            ));
            assert!(fixture.expected().is_empty());
        }
    }
    assert_eq!(populated, 1);
}

#[test]
fn f1_independent_arithmetic_and_two_logical_builds_agree() {
    let fixture = f1();
    let first = fixture.build_db(true).unwrap();
    let second = fixture.build_db(true).unwrap();
    assert_ne!(first.path(), second.path());
    let session_id = &fixture.sessions()[0].metadata.session_id;
    assert_eq!(
        first.store().session(session_id).unwrap(),
        second.store().session(session_id).unwrap()
    );
    let rows = first.store().records(session_id).unwrap();
    assert_eq!(rows, second.store().records(session_id).unwrap());
    assert_eq!(rows.len(), 25);
    assert_eq!(rows.iter().map(|r| r.tool_uses.len()).sum::<usize>(), 5);
    assert_eq!(first.store().counts().unwrap().usage_rows, 15);
    // Five turns each contain a 135-token draft, a 140-token final observation
    // of the same response, and an 80-token second response. Summing every row
    // overcounts; independently selecting the final observations gives 1,100.
    let naive_total: i64 = rows
        .iter()
        .filter_map(|r| r.usage.as_ref())
        .map(|u| {
            u.input_tokens.unwrap()
                + u.output_tokens.unwrap()
                + u.cache_read_input_tokens.unwrap()
                + u.cache_creation_input_tokens.unwrap()
        })
        .sum();
    assert_eq!(naive_total, 1775);
    let selected_total = rows
        .iter()
        .filter(|r| {
            r.content_json
                .as_ref()
                .unwrap()
                .as_array()
                .unwrap()
                .iter()
                .any(|b| {
                    b["type"] == "tool_use"
                        || b["text"]
                            .as_str()
                            .is_some_and(|t| t.starts_with("Completed synthetic task"))
                })
        })
        .filter_map(|r| r.usage.as_ref())
        .map(|u| {
            u.input_tokens.unwrap()
                + u.output_tokens.unwrap()
                + u.cache_read_input_tokens.unwrap()
                + u.cache_creation_input_tokens.unwrap()
        })
        .sum::<i64>();
    assert_eq!(selected_total, 5 * ((100 + 10 + 20 + 10) + (50 + 20 + 10)));
    fixture.assert_expectation("M-04", &json!({"selected_responses":10,"input_tokens":750,"output_tokens":150,"cache_read_input_tokens":150,"cache_creation_input_tokens":50,"total_tokens":selected_total})).unwrap();
    assert_eq!(
        rows.last().unwrap().ts_ms.unwrap() - rows.first().unwrap().ts_ms.unwrap(),
        23 * 60 * 1000
    );
    fixture.assert_reference().unwrap();
}

#[test]
fn wal_reader_snapshot_survives_a_writer_and_owner_cleans_up() {
    let fixture = f1();
    let mut owner = fixture.build_db(true).unwrap();
    let path = owner.path().to_path_buf();
    let reader = Connection::open(&path).unwrap();
    let journal: String = reader
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .unwrap();
    assert_eq!(journal, "wal");
    reader.execute_batch("BEGIN").unwrap();
    let count = || {
        reader
            .query_row::<i64, _, _>("SELECT count(*) FROM records", [], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(count(), 25);
    let mut next = fixture.sessions()[0].records[0].clone();
    next.uuid = Some("22222222-2222-4222-8222-000000000001".into());
    owner
        .store_mut()
        .upsert_records(&fixture.sessions()[0].metadata.session_id, &[next], true)
        .unwrap();
    assert_eq!(count(), 25, "reader retains the pre-commit snapshot");
    reader.execute_batch("COMMIT").unwrap();
    assert_eq!(
        count(),
        26,
        "reader sees committed WAL after ending snapshot"
    );
    let other_store = Store::open(&path).unwrap();
    assert_eq!(other_store.counts().unwrap().records, 26);
    drop(other_store);
    drop(reader);
    assert!(path.is_file());
    drop(owner);
    assert!(!path.exists());
}

#[test]
fn database_export_is_complete_closed_and_never_clobbers() {
    let fixture = f1();
    let directory = TempDir::new().unwrap();
    let path = directory.path().join("F1.sqlite");
    fixture.write_db(&path, true).unwrap();
    let original = fs::read(&path).unwrap();
    assert!(original.starts_with(b"SQLite format 3\0"));
    assert!(!directory.path().join("F1.sqlite-wal").exists());
    let store = Store::open(&path).unwrap();
    assert_eq!(store.counts().unwrap().records, 25);
    assert_eq!(store.counts().unwrap().usage_rows, 15);
    drop(store);
    assert!(fixture.write_db(&path, true).is_err());
    assert_eq!(fs::read(&path).unwrap(), original);
    let blocked = directory.path().join("blocked.sqlite");
    fs::write(
        directory.path().join("blocked.sqlite-wal"),
        "owned elsewhere",
    )
    .unwrap();
    assert!(fixture.write_db(&blocked, true).is_err());
    assert!(!blocked.exists());
}

#[test]
fn build_retention_mode_is_explicit_without_changing_counts() {
    let fixture = f1();
    let full = fixture.build_db(true).unwrap();
    let metadata = fixture.build_db(false).unwrap();
    assert_eq!(
        full.store().counts().unwrap(),
        metadata.store().counts().unwrap()
    );
    let rows = metadata
        .store()
        .records(&fixture.sessions()[0].metadata.session_id)
        .unwrap();
    assert!(rows.iter().all(|r| r.content_json.is_none() && r.tool_uses.iter().all(|t| t.input_json.is_none())));
    assert_eq!(rows.iter().map(|r| r.tool_uses.len()).sum::<usize>(), 5);
}

#[test]
fn custom_manifest_paths_and_absent_optional_identity_are_supported() {
    let directory = editable("F1");
    let custom = directory.path().join("input/custom");
    fs::create_dir(&custom).unwrap();
    fs::rename(
        directory.path().join("input/sessions/session.jsonl"),
        custom.join("native.events"),
    )
    .unwrap();
    edit_json(&directory.path().join("manifest.json"), |m| {
        let session = m["sessions"][0].as_object_mut().unwrap();
        session.insert("file".into(), json!("input/custom/native.events"));
        for field in ["source_surface", "native_session_id", "started_at"] {
            session.remove(field);
        }
    });
    let fixture = Fixture::load(directory.path()).unwrap();
    assert_eq!(fixture.sessions()[0].records.len(), 25);
    assert!(fixture.sessions()[0].metadata.surface.is_none());
    assert!(fixture.sessions()[0].metadata.started_at_ms.is_none());
    fixture.assert_reference().unwrap();
}

#[test]
fn optional_record_timestamp_preserves_unknown_in_canonical_export() {
    let directory = editable("F1");
    edit_record(directory.path(), 0, |r| {
        r.as_object_mut().unwrap().remove("timestamp");
    });
    let fixture = Fixture::load(directory.path()).unwrap();
    let db = fixture.build_db(true).unwrap();
    assert!(
        db.store()
            .records(&fixture.sessions()[0].metadata.session_id)
            .unwrap()
            .iter()
            .any(|r| r.ts_ms.is_none())
    );
    assert!(
        fixture.assert_reference().is_err(),
        "F1 reference requires measured timing even though the shared loader does not"
    );
}

#[test]
fn canonical_export_round_trips_the_loaded_inputs_and_auxiliary_files() {
    let fixture = f1();
    let export = serde_json::to_value(fixture.export()).unwrap();
    let sessions: Vec<LoadedSession> = serde_json::from_value(export["sessions"].clone()).unwrap();
    assert_eq!(sessions, fixture.sessions());
    assert_eq!(
        export["expected"],
        serde_json::to_value(fixture.expected()).unwrap()
    );
    let probes = Fixture::load(catalog().join("F16")).unwrap();
    assert_eq!(
        probes
            .snapshots()
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>(),
        ["fresh", "typical", "connected", "no-python3", "env"]
            .into_iter()
            .collect()
    );
    assert!(
        probes
            .snapshots()
            .values()
            .all(|v| v["status"] == "skeleton")
    );
    let gh = Fixture::load(catalog().join("F4")).unwrap();
    assert_eq!(gh.gh().unwrap()["status"], "skeleton");
    assert_eq!(
        serde_json::to_value(gh.export()).unwrap()["manifest"]["status"],
        "skeleton"
    );
}

#[test]
fn all_defined_keys_round_trip_as_schema_not_product_acceptance() {
    let mut defined = BTreeSet::new();
    for (prefix, count) in [("M", 19), ("C", 8), ("O", 13), ("P", 2), ("R", 8), ("U", 8)] {
        for number in 1..=count {
            defined.insert(format!("{prefix}-{number:02}"));
        }
    }
    defined.extend(["M-11a".to_owned(), "M-12a".to_owned()]);
    assert_eq!(defined.len(), 60);
    assert_eq!(
        RULE_IDS
            .iter()
            .map(|s| s.to_string())
            .collect::<BTreeSet<_>>(),
        defined
    );
    let directory = editable("F1");
    edit_json(&directory.path().join("manifest.json"), |m| {
        m["proves"] = json!(defined)
    });
    let expected = defined
        .iter()
        .map(|key| (key.clone(), Value::Null))
        .collect::<serde_json::Map<_, _>>();
    fs::write(
        directory.path().join("expected.json"),
        serde_json::to_vec(&expected).unwrap(),
    )
    .unwrap();
    let fixture = Fixture::load(directory.path()).unwrap();
    let export = serde_json::to_value(fixture.export()).unwrap();
    for key in &defined {
        assert!(RuleId::parse(key).is_ok());
        assert!(export["expected"].get(key).is_some());
        fixture.assert_expectation(key, &Value::Null).unwrap();
        assert!(fixture.assert_expectation(key, &json!(0)).is_err());
    }
    assert!(
        fixture.assert_reference().is_err(),
        "schema acceptance does not implement sixty rules"
    );
}

#[test]
fn unknown_rule_ids_in_either_manifest_or_goldens_fail_with_locations() {
    for key in ["M-99", "M-11b", "P-03", "E-01"] {
        for file in ["manifest.json", "expected.json"] {
            let directory = editable("F1");
            edit_json(&directory.path().join(file), |value| {
                if file == "manifest.json" {
                    value["proves"][0] = json!(key);
                } else {
                    value[key] = Value::Null;
                }
            });
            let error = load_error(directory.path());
            assert!(
                error.contains(file) && error.contains("unknown rule key"),
                "{error}"
            );
        }
    }
}

#[test]
fn duplicate_golden_keys_fail_before_identical_or_conflicting_values_are_lost() {
    for (key, identical) in [
        (r#""M-04""#, true),
        (r#""M-04""#, false),
        (r#""M-\u0030\u0034""#, false),
    ] {
        let directory = editable("F1");
        let path = directory.path().join("expected.json");
        let original = fs::read_to_string(&path).unwrap();
        let parsed: Value = serde_json::from_str(&original).unwrap();
        let first_value = if identical {
            parsed["M-04"].clone()
        } else {
            Value::Null
        };
        // Write literal JSON entries: constructing a Value/map here would erase
        // the duplicate before the loader under test ever receives it.
        let duplicated = format!(
            "{{{key}:{first_value},{}",
            original.trim_start().strip_prefix('{').unwrap()
        );
        fs::write(path, duplicated).unwrap();
        let error = load_error(directory.path());
        assert!(
            error.contains("expected.json:") && error.contains("duplicate golden key M-04"),
            "{error}"
        );
    }
}

#[test]
fn nested_duplicate_golden_fields_fail_before_assertions() {
    for repeated in [
        r#""total_tokens": 1775, "total_tokens": 1100"#,
        r#""total_tokens": 1100, "total_tokens": 1100"#,
        r#""total_\u0074okens": 1775, "total_tokens": 1100"#,
        r#""total_tokens": {"nested": {"count": 1, "count": 2}}"#,
        r#""total_tokens": [{"nested": [{"count": 1, "count": 2}]}]"#,
    ] {
        let directory = editable("F1");
        let path = directory.path().join("expected.json");
        let original = fs::read_to_string(&path).unwrap();
        assert!(original.contains(r#""total_tokens": 1100"#));
        fs::write(path, original.replace(r#""total_tokens": 1100"#, repeated)).unwrap();
        let error = load_error(directory.path());
        assert!(
            error.contains("expected.json:") && error.contains("duplicate golden field"),
            "{error}"
        );
    }
}

#[test]
fn recursive_goldens_preserve_json_types_and_separate_object_keys() {
    let directory = editable("F1");
    let value = json!({
        "values": [null, true, false, -7, u64::MAX, 1.25, "synthetic", [], {}],
        "objects": [{"count": 1}, {"count": 2}],
        "nested": {"count": {"count": 3}}
    });
    edit_json(&directory.path().join("expected.json"), |expected| {
        expected["M-04"] = value.clone();
    });
    let fixture = Fixture::load(directory.path()).unwrap();
    assert_eq!(
        serde_json::to_value(fixture.export()).unwrap()["expected"]["M-04"],
        value
    );
}

#[test]
fn missing_inputs_and_invalid_uuid_or_timestamp_identify_source_location() {
    let directory = editable("F1");
    fs::remove_file(directory.path().join("input/sessions/session.jsonl")).unwrap();
    assert!(load_error(directory.path()).contains("input/sessions/session.jsonl"));
    for (field, value, location) in [
        ("uuid", json!("not-a-uuid"), "session.jsonl:1:uuid"),
        (
            "timestamp",
            json!("2026-02-30T12:00:00Z"),
            "session.jsonl:1:timestamp",
        ),
        (
            "timestamp",
            json!("2026-09-07 12:00:00"),
            "session.jsonl:1:timestamp",
        ),
    ] {
        let directory = editable("F1");
        edit_record(directory.path(), 0, |r| r[field] = value);
        assert!(load_error(directory.path()).contains(location));
    }
    for field in ["now", "started_at"] {
        let directory = editable("F1");
        edit_json(&directory.path().join("manifest.json"), |m| {
            if field == "now" {
                m[field] = json!("yesterday");
            } else {
                m["sessions"][0][field] = json!("yesterday");
            }
        });
        let error = load_error(directory.path());
        assert!(
            error.contains(field) && error.contains("invalid RFC3339"),
            "{error}"
        );
    }
}

#[test]
fn skeletons_cannot_use_null_as_an_unimplemented_golden() {
    let directory = editable("F2");
    edit_json(&directory.path().join("expected.json"), |e| {
        e["M-05"] = Value::Null
    });
    assert!(load_error(directory.path()).contains("null means unmeasured"));
    let directory = editable("F2");
    edit_json(&directory.path().join("manifest.json"), |m| {
        m["status"] = json!("populated")
    });
    assert!(load_error(directory.path()).contains("populated expectations must match"));
}

#[test]
fn bad_input_and_bad_goldens_both_break_reference_assertions() {
    let wrong_golden = editable("F1");
    edit_json(&wrong_golden.path().join("expected.json"), |e| {
        e["M-04"]["total_tokens"] = json!(1775)
    });
    let error = Fixture::load(wrong_golden.path())
        .unwrap()
        .assert_reference()
        .unwrap_err()
        .to_string();
    assert!(error.contains("F1:expected:M-04"));
    let wrong_input = editable("F1");
    edit_record(wrong_input.path(), 2, |r| {
        r["message"]["usage"]["output_tokens"] = json!(11)
    });
    let error = Fixture::load(wrong_input.path())
        .unwrap()
        .assert_reference()
        .unwrap_err()
        .to_string();
    assert!(error.contains("F1:expected:M-04"));
}

#[test]
fn declared_paths_cannot_escape_the_fixture() {
    for file in ["../outside.jsonl", "/outside.jsonl"] {
        let directory = editable("F1");
        edit_json(&directory.path().join("manifest.json"), |m| {
            m["sessions"][0]["file"] = json!(file)
        });
        assert!(load_error(directory.path()).contains("relative path without traversal"));
    }
    #[cfg(unix)]
    {
        let directory = editable("F1");
        let outside = tempfile::NamedTempFile::new().unwrap();
        let path = directory.path().join("input/sessions/session.jsonl");
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(outside.path(), path).unwrap();
        assert!(load_error(directory.path()).contains("inside the fixture directory"));
    }
}

#[test]
fn catalog_rejects_zero_padding_missing_entries_and_mismatched_ids() {
    assert!(FixtureId::parse("F01").is_err());
    assert!(FixtureId::parse("F22").is_err());
    let directory = TempDir::new().unwrap();
    copy_tree(&catalog(), directory.path());
    fs::create_dir(directory.path().join("F01")).unwrap();
    assert!(
        Fixture::all(directory.path())
            .err()
            .unwrap()
            .to_string()
            .contains("zero padding")
    );
    fs::remove_dir(directory.path().join("F01")).unwrap();
    edit_json(&directory.path().join("F2/manifest.json"), |m| {
        m["id"] = json!("F3")
    });
    assert!(
        Fixture::all(directory.path())
            .err()
            .unwrap()
            .to_string()
            .contains("does not match")
    );
    fs::remove_dir_all(directory.path().join("F2")).unwrap();
    assert!(
        Fixture::all(directory.path())
            .err()
            .unwrap()
            .to_string()
            .contains("F2")
    );
}

#[test]
fn repeated_uuids_remain_available_for_replay_tests_but_store_deduplicates() {
    let directory = editable("F1");
    let path = directory.path().join("input/sessions/session.jsonl");
    let input = fs::read_to_string(&path).unwrap();
    fs::write(&path, input.clone() + &input).unwrap();
    let fixture = Fixture::load(directory.path()).unwrap();
    assert_eq!(fixture.sessions()[0].records.len(), 50);
    let db = fixture.build_db(true).unwrap();
    assert_eq!(db.store().counts().unwrap().records, 25);
    fixture.assert_reference().unwrap();
}

#[test]
fn raw_platform_is_preserved_and_known_host_mismatches_fail() {
    let directory = editable("F1");
    edit_json(&directory.path().join("manifest.json"), |m| {
        m["sessions"][0]["host"] = json!("other");
        m["sessions"][0]["source_platform"] = json!("future-agent");
    });
    let fixture = Fixture::load(directory.path()).unwrap();
    let db = fixture.build_db(false).unwrap();
    let stored = db
        .store()
        .session(&fixture.sessions()[0].metadata.session_id)
        .unwrap()
        .unwrap();
    assert_eq!(stored.meta.host, xt_store::Host::Other);
    assert_eq!(stored.meta.source_platform.as_deref(), Some("future-agent"));
    edit_json(&directory.path().join("manifest.json"), |m| {
        m["sessions"][0]["source_platform"] = json!("codex")
    });
    assert!(load_error(directory.path()).contains("source_platform must map"));
}

#[test]
fn absent_raw_platform_stays_unknown_independently_of_declared_host() {
    use xt_store::Host;
    for (host, raw) in [
        (Host::Claude, None),
        (Host::Codex, None),
        (Host::Cursor, None),
        (Host::Other, None),
        (Host::Claude, Some("claude")),
        (Host::Other, Some("future-agent")),
    ] {
        let directory = editable("F1");
        edit_json(&directory.path().join("manifest.json"), |m| {
            m["sessions"][0]["host"] = json!(host);
            if let Some(raw) = raw {
                m["sessions"][0]["source_platform"] = json!(raw);
            } else {
                m["sessions"][0]
                    .as_object_mut()
                    .unwrap()
                    .remove("source_platform");
            }
        });
        let fixture = Fixture::load(directory.path()).unwrap();
        let metadata = &fixture.sessions()[0].metadata;
        assert_eq!(metadata.host, host);
        assert_eq!(metadata.source_platform.as_deref(), raw);
        let export = serde_json::to_value(fixture.export()).unwrap();
        assert_eq!(export["sessions"][0]["metadata"]["host"], json!(host));
        assert_eq!(
            export["sessions"][0]["metadata"]["source_platform"],
            json!(raw)
        );
        let db = fixture.build_db(false).unwrap();
        let stored = db.store().session(&metadata.session_id).unwrap().unwrap();
        assert_eq!(stored.meta.host, host);
        assert_eq!(stored.meta.source_platform.as_deref(), raw);
    }
}

#[test]
fn skeleton_rejects_records_in_any_declared_session() {
    let canonical = fs::read_to_string(catalog().join("F1/input/sessions/session.jsonl")).unwrap();
    for populated in [[true, false], [false, true], [true, true]] {
        let directory = editable("F2");
        edit_json(&directory.path().join("manifest.json"), |manifest| {
            let mut second = manifest["sessions"][0].clone();
            second["session_id"] = json!("00000000-0000-4000-8000-000000000099");
            second["file"] = json!("input/sessions/second.jsonl");
            manifest["sessions"].as_array_mut().unwrap().push(second);
        });
        let paths = [
            "input/sessions/session.jsonl",
            "input/sessions/second.jsonl",
        ];
        for path in paths {
            fs::write(directory.path().join(path), " \n\n").unwrap();
        }
        assert_eq!(
            Fixture::load(directory.path()).unwrap().manifest().status,
            FixtureStatus::Skeleton
        );
        for (path, has_records) in paths.into_iter().zip(populated) {
            if has_records {
                fs::write(
                    directory.path().join(path),
                    canonical.lines().next().unwrap(),
                )
                .unwrap();
            }
        }
        let error = load_error(directory.path());
        assert!(error.contains("manifest.json"));
        assert!(error.contains("skeleton canonical inputs must be empty"));
    }
}
