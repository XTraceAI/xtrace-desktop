use serde::Deserialize;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};
use xt_fixtures::Fixture;
use xt_ingest::canonical::{DropReason, Parsed, SourceContext, parse_line, parse_with_context};

#[derive(Deserialize)]
struct Case {
    name: String,
    line: Value,
    expected_variant: String,
    #[serde(default)]
    context: SourceContext,
}

fn cases() -> Vec<Case> {
    ["F17", "F18", "F20"]
        .into_iter()
        .flat_map(|id| {
            let fixture = Fixture::load(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../fixtures")
                    .join(id),
            )
            .unwrap();
            serde_json::from_value::<Vec<Case>>(fixture.snapshots()["parser"]["cases"].clone())
                .unwrap()
        })
        .collect()
}

fn parse_case(name: &str) -> Parsed {
    let case = cases().into_iter().find(|case| case.name == name).unwrap();
    parse_with_context(&case.line.to_string(), &case.context).unwrap()
}

fn record(name: &str) -> Box<xt_ingest::canonical::ParsedRecord> {
    let Parsed::Record(record) = parse_case(name) else {
        panic!("expected record");
    };
    record
}

fn variant(value: &Parsed) -> &'static str {
    match value {
        Parsed::Record(_) => "record",
        Parsed::StructuralEvent(_) => "structural",
        Parsed::PrLink(_) => "pr_link",
        Parsed::Inert => "inert",
        Parsed::Dropped(_) => "dropped",
    }
}

#[test]
fn canonical_and_structural_shapes_match_the_shared_fixture_catalog() {
    let all = cases();
    assert_eq!(all.len(), 20);
    for case in all {
        let parsed = parse_with_context(&case.line.to_string(), &case.context).unwrap();
        assert_eq!(variant(&parsed), case.expected_variant, "{}", case.name);
    }
}

#[test]
fn canonical_response_identity_cache_and_native_ancestry_survive() {
    let first = record("claude-cache-sidechain");
    let second = record("same-response-tool");
    assert_ne!(first.canonical.uuid, second.canonical.uuid);
    assert_eq!(
        first.canonical.api_message_id.as_deref(),
        Some("response-1")
    );
    assert_eq!(
        first.canonical.api_message_id,
        second.canonical.api_message_id
    );
    assert_eq!(first.canonical.request_id, second.canonical.request_id);
    assert_eq!(
        first.canonical.timestamp.as_deref(),
        Some("2026-09-07T12:00:00.000000000123Z")
    );
    assert!(first.canonical.is_sidechain);
    assert_eq!(first.native.agent_id.as_deref(), Some("agent-sidechain"));
    assert_eq!(
        first.native.parent_uuid.as_deref(),
        Some("02000000-0000-4000-8000-000000000000")
    );
    assert_eq!(first.native.session_id.as_deref(), Some("native-claude"));
    let usage = first.canonical.message.usage.as_ref().unwrap();
    assert_eq!(usage.cache_read_input_tokens, Some(0));
    assert_eq!(
        usage
            .cache_creation
            .as_ref()
            .unwrap()
            .ephemeral_1h_input_tokens,
        Some(12)
    );
    assert_eq!(usage.service_tier.as_deref(), Some("standard"));
    let codex = record("codex-canonical-usage");
    assert_eq!(
        codex.canonical.message.usage.unwrap().output_tokens,
        Some(12),
        "reasoning detail is not added again"
    );
}

#[test]
fn canonical_optional_values_and_measured_empty_content_stay_distinct() {
    for name in ["minimal-user", "reader-header-context"] {
        let parsed = record(name);
        assert!(parsed.canonical.message.content.is_none());
        assert!(parsed.canonical.message.usage.is_none());
        assert!(parsed.canonical.message.model.is_none());
        assert!(parsed.canonical.timestamp.is_none());
        assert_eq!(parsed.tool_use_count, None);
        assert_eq!(parsed.is_tool_result_carrier, None);
    }
    let missing_ids = record("missing-response-ids");
    assert_eq!(missing_ids.canonical.api_message_id, None);
    assert_eq!(missing_ids.canonical.request_id, None);
    assert_eq!(missing_ids.tool_use_count, Some(0));
    let empty = record("measured-empty-string");
    assert_eq!(
        empty.canonical.message.content,
        Some(vec![json!({"type":"text","text":""})])
    );
    assert_eq!(empty.tool_use_count, Some(0));
    assert_eq!(empty.is_tool_result_carrier, Some(false));
    let cursor = record("cursor-unknown-measurements");
    assert_eq!(cursor.canonical.message.usage, None);
    assert_eq!(cursor.canonical.message.model, None);
    assert_eq!(cursor.canonical.timestamp, None);
    assert_eq!(cursor.canonical.cwd, None);
}

#[test]
fn canonical_line_header_and_entrypoint_identities_are_preserved_independently() {
    let entrypoint = record("future-entrypoint");
    assert_eq!(
        entrypoint.canonical.source_surface.as_deref(),
        Some("future.desktop.v3")
    );
    assert_eq!(
        entrypoint.native.entrypoint.as_deref(),
        Some("future.desktop.v3")
    );
    let distinct = record("distinct-line-identities");
    assert_eq!(
        distinct.canonical.source_surface.as_deref(),
        Some("wire-surface")
    );
    assert_eq!(distinct.native.entrypoint.as_deref(), Some("native-entry"));
    assert_eq!(
        distinct.canonical.native_session_id.as_deref(),
        Some("wire-native")
    );
    assert_eq!(distinct.native.session_id.as_deref(), Some("raw-native"));
    assert_eq!(
        distinct.source.conversation_id.as_deref(),
        Some("wire-canonical")
    );
    assert_eq!(
        distinct.source.source_platform.as_deref(),
        Some("future-agent")
    );
    let header = record("reader-header-context");
    assert_eq!(
        header.context.conversation_id.as_deref(),
        Some("cursor-canonical")
    );
    assert_eq!(
        header.context.native_session_id.as_deref(),
        Some("cursor-native")
    );
    assert_eq!(
        header.context.source_surface.as_deref(),
        Some("future.header")
    );
    assert_eq!(
        header.context.started_at.as_deref(),
        Some("2026-09-07T11:00:00Z")
    );
    assert_eq!(
        header.canonical.timestamp, None,
        "native start never becomes event time"
    );
    assert_eq!(
        header.source,
        SourceContext::default(),
        "header evidence does not become line evidence"
    );
}

#[test]
fn canonical_pr_links_need_no_uuid_and_hook_summaries_are_content_free_events() {
    let Parsed::PrLink(link) = parse_case("native-pr-link-no-uuid") else {
        panic!("expected PR link");
    };
    assert_eq!(link.number, 42);
    assert_eq!(link.raw_repository, "example/fixture");
    assert_eq!(link.raw_url, "https://github.com/example/fixture/pull/42");
    assert_eq!(link.native.session_id.as_deref(), Some("native-claude"));
    let Parsed::StructuralEvent(summary) = parse_case("native-stop-hook-summary") else {
        panic!("expected structural summary");
    };
    assert_eq!(summary.hook_count, Some(3));
    assert_eq!(summary.prevented_continuation, Some(false));
    assert_eq!(summary.total_duration_ms, Some(15));
    assert_eq!(summary.tool_use_id.as_deref(), Some("tool-1"));
    assert!(summary.uuid.is_some());
    let serialized = serde_json::to_string(&summary).unwrap();
    assert!(!serialized.contains("synthetic-private-"));
    assert!(!serialized.contains("hookInfos"));
    let Parsed::StructuralEvent(unknown) = parse_case("unknown-summary-time") else {
        panic!("expected structural summary");
    };
    assert_eq!(unknown.timestamp, None);
    assert_eq!(
        unknown.source.native_session_id.as_deref(),
        Some("line-native")
    );
    assert_eq!(
        unknown.context.native_session_id.as_deref(),
        Some("header-native")
    );
    assert_eq!(unknown.native.session_id.as_deref(), Some("raw-native"));
    assert_eq!(
        unknown.source.source_surface.as_deref(),
        Some("native-surface")
    );
    assert_eq!(
        unknown.context.source_surface.as_deref(),
        Some("header-surface")
    );
}

#[test]
fn canonical_classification_remains_after_transient_content_is_discarded() {
    let mut tool = record("same-response-tool");
    tool.canonical.message.content = None;
    assert_eq!(tool.tool_use_count, Some(1));
    let mut carrier = record("tool-result-carrier");
    carrier.canonical.message.content = None;
    assert_eq!(carrier.tool_use_count, Some(0));
    assert_eq!(carrier.is_tool_result_carrier, Some(true));
}

#[test]
fn canonical_missing_uuid_is_countable_and_unknown_types_are_inert() {
    for input in [
        json!({"type":"user"}),
        json!({"type":"assistant","uuid":null}),
        json!({"type":"user","uuid":" ","message":false}),
    ] {
        assert_eq!(
            parse_line(&input.to_string()).unwrap(),
            Parsed::Dropped(DropReason::MissingUuid)
        );
    }
    for input in [
        json!({}),
        json!({"type":"new-kind","timestamp":42}),
        json!({"type":"system","subtype":"future","message":false}),
    ] {
        assert_eq!(parse_line(&input.to_string()).unwrap(), Parsed::Inert);
    }
}

#[test]
fn canonical_known_malformed_inputs_have_bounded_payload_free_errors() {
    let mut invalid = vec![
        "not JSON".to_string(),
        "[]".to_string(),
        "{\"type\":42}".to_string(),
    ];
    for change in [
        json!({"timestamp":"2026-02-30T00:00:00Z"}),
        json!({"message":42}),
        json!({"message":{"content":[{"type":"text","text":false}]}}),
        json!({"message":{"content":[{"type":"tool_use","name":" "}]}}),
        json!({"message":{"usage":{"input_tokens":-1}}}),
        json!({"message":{"usage":{"output_tokens":1.5}}}),
        json!({"agentId":42}),
    ] {
        let mut input = json!({"type":"user","uuid":"valid-synthetic","message":{}});
        input
            .as_object_mut()
            .unwrap()
            .extend(change.as_object().unwrap().clone());
        input["private-marker"] = json!("synthetic-sensitive-value".repeat(1000));
        invalid.push(input.to_string());
    }
    invalid.push(json!({"type":"pr-link","prNumber":0,"prUrl":"x","prRepository":"x"}).to_string());
    invalid.push(json!({"type":"system","subtype":"stop_hook_summary","hookCount":-1}).to_string());
    for input in invalid {
        let error = parse_line(&input).unwrap_err();
        for text in [error.to_string(), format!("{error:?}")] {
            assert!(text.len() < 128);
            assert!(!text.contains("synthetic-sensitive-value"));
        }
    }
}

#[test]
fn canonical_parse_50k() {
    let dataset = cases()
        .into_iter()
        .map(|case| (case.line.to_string(), case.context))
        .collect::<Vec<_>>();
    assert_eq!(dataset.len(), 20);
    // Hardware discovery and fixture I/O occur outside the measured parser loop.
    let hardware = if cfg!(target_os = "macos") {
        std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        None
    }
    .unwrap_or_else(|| {
        format!(
            "{} {} {} threads",
            std::env::consts::OS,
            std::env::consts::ARCH,
            std::thread::available_parallelism().map_or(1, usize::from)
        )
    });
    let start = Instant::now();
    let mut counts = [0usize; 5];
    for (line, context) in dataset.iter().cycle().take(50_000) {
        let parsed = parse_with_context(std::hint::black_box(line), context).unwrap();
        counts[match parsed {
            Parsed::Record(_) => 0,
            Parsed::StructuralEvent(_) => 1,
            Parsed::PrLink(_) => 2,
            Parsed::Inert => 3,
            Parsed::Dropped(_) => 4,
        }] += 1;
        std::hint::black_box(parsed);
    }
    let elapsed = start.elapsed();
    assert_eq!(counts, [30_000, 5_000, 2_500, 7_500, 5_000]);
    println!(
        "dataset=canonical-native-v1:F17+F18+F20:20-lines count=50000 records={} structural={} pr_links={} inert={} dropped={} elapsed_ms={} runner={} profile={}",
        counts[0],
        counts[1],
        counts[2],
        counts[3],
        counts[4],
        elapsed.as_millis(),
        hardware,
        if cfg!(debug_assertions) {
            "debug (timing gate disabled)"
        } else {
            "release"
        }
    );
    if !cfg!(debug_assertions) {
        assert!(
            elapsed.as_secs_f64() < 1.0,
            "50k parser budget exceeded: {elapsed:?}, {hardware}; excludes fixture/hardware I/O and includes result allocation/drop"
        );
    }
}
