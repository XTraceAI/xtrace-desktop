//! Independent F1 arithmetic over persisted rows. Intentionally limited to its
//! one-Claude-session, one-model, fully timed and measured shape. Production
//! metric queries, other host adapters, prices and rule edge cases live elsewhere.
use crate::{Fixture, Result, invalid};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use xt_store::{Host, Store, StoredRecord};

pub(crate) fn assert_rows(fixture: &Fixture, store: &Store) -> Result<()> {
    let sessions = fixture.sessions();
    if sessions.len() != 1 || sessions[0].metadata.host != Host::Claude {
        return Err(invalid(
            "F1",
            "reference assertions require one Claude session",
        ));
    }
    let session = store
        .session(&sessions[0].metadata.session_id)?
        .ok_or_else(|| invalid("F1", "reference session must exist in the database"))?;
    // Record metadata can mark the session conflicted without flagging any row.
    if session.meta.host != Host::Claude || session.has_conflict {
        return Err(invalid(
            "F1",
            "reference session must be Claude and have no conflicts",
        ));
    }
    let rows = store.records(&session.meta.session_id)?;
    if rows.is_empty()
        || rows
            .iter()
            .any(|r| r.has_conflict || r.ts_ms.is_none() || r.content_json.is_none())
    {
        return Err(invalid(
            "F1",
            "reference rows must have timestamps, content and no conflicts",
        ));
    }
    let start = fixture.window_start().timestamp_millis();
    let end = fixture.now().timestamp_millis();
    if rows
        .iter()
        .any(|r| !(start..end).contains(&r.ts_ms.unwrap()))
    {
        return Err(invalid(
            "F1",
            "reference inputs must all be in the fixed window",
        ));
    }
    let human = rows.iter().map(is_human).collect::<Vec<_>>();
    fixture.assert_expectation(
        "M-02",
        &json!({"human_messages": human.iter().filter(|&&v| v).count()}),
    )?;

    let mut turns = 0;
    let mut after_human = false;
    let mut in_response = false;
    for &is_human in &human {
        if is_human {
            after_human = true;
            in_response = false;
        } else if after_human && !in_response {
            turns += 1;
            in_response = true;
        }
    }
    fixture.assert_expectation("M-03", &json!({
        "assistant_turns": turns,
        "assistant_records": rows.iter().filter(|r| r.role.as_deref() == Some("assistant")).count(),
        "tool_result_carriers": rows.iter().filter(|r| r.is_tool_result_carrier == Some(true)).count(),
        "tool_calls": rows.iter().map(|r| r.tool_uses.len()).sum::<usize>(),
        "records": rows.len()
    }))?;

    // Select cumulative Claude response observations before summing. Content and
    // tool rows remain stored; only usage is reduced to one latest observation.
    let mut responses: BTreeMap<(&str, &str), &StoredRecord> = BTreeMap::new();
    for row in rows
        .iter()
        .filter(|r| r.role.as_deref() == Some("assistant"))
    {
        let ids = row
            .api_message_id
            .as_deref()
            .zip(row.request_id.as_deref())
            .filter(|(message, request)| !message.is_empty() && !request.is_empty())
            .ok_or_else(|| invalid("F1:M-04", "reference responses require both native IDs"))?;
        if row.usage.is_none() || row.model.as_deref() == Some("<synthetic>") || row.is_sidechain {
            return Err(invalid("F1:M-04", "unsupported reference response shape"));
        }
        if let Some(previous) = responses.get(&ids) {
            if previous.ts_ms == row.ts_ms {
                return Err(invalid(
                    "F1:M-04",
                    "reference input must not require equal-timestamp tie-breaking",
                ));
            }
            if previous.ts_ms > row.ts_ms {
                continue;
            }
        }
        responses.insert(ids, row);
    }
    let models = responses
        .values()
        .filter_map(|r| r.model.as_deref())
        .collect::<BTreeSet<_>>();
    if models.len() != 1 || responses.values().any(|r| r.model.is_none()) {
        return Err(invalid(
            "F1:M-10",
            "reference responses require exactly one measured model",
        ));
    }
    let mut counters = [0_i64; 4];
    for row in responses.values() {
        let usage = row.usage.as_ref().unwrap();
        for (total, counter) in counters.iter_mut().zip([
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_input_tokens,
            usage.cache_creation_input_tokens,
        ]) {
            *total =
                total
                    .checked_add(counter.ok_or_else(|| {
                        invalid("F1:M-04", "reference counters must all be measured")
                    })?)
                    .ok_or_else(|| invalid("F1:M-04", "counter overflow"))?;
        }
    }
    let total_tokens = counters
        .iter()
        .try_fold(0_i64, |sum, value| sum.checked_add(*value))
        .ok_or_else(|| invalid("F1:M-04", "counter overflow"))?;
    fixture.assert_expectation("M-04", &json!({
        "selected_responses": responses.len(), "input_tokens": counters[0], "output_tokens": counters[1],
        "cache_read_input_tokens": counters[2], "cache_creation_input_tokens": counters[3], "total_tokens": total_tokens
    }))?;

    // Sorted stored rows form activity spans; a gap strictly over twenty minutes
    // starts a new span and is excluded. F1's single span exercises the baseline.
    let active_ms = rows
        .windows(2)
        .map(|pair| pair[1].ts_ms.unwrap() - pair[0].ts_ms.unwrap())
        .filter(|gap| *gap <= 20 * 60 * 1000)
        .sum::<i64>();
    fixture.assert_expectation("M-05", &json!({"active_ms": active_ms}))?;
    fixture.assert_expectation(
        "M-10",
        &json!({"favorite_model": models.first().unwrap(), "output_tokens": counters[1]}),
    )?;
    if fixture.expected().len() != 5 {
        return Err(invalid(
            "F1",
            "additional golden keys need executable reference assertions",
        ));
    }
    Ok(())
}

fn is_human(row: &StoredRecord) -> bool {
    let text = row
        .content_json
        .as_ref()
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|b| b.get("type").and_then(|v| v.as_str()) == Some("text"))
        .filter_map(|b| b.get("text").and_then(|v| v.as_str()))
        .collect::<String>();
    row.role.as_deref() == Some("user")
        && row.is_tool_result_carrier == Some(false)
        && !row.is_meta
        && !row.is_sidechain
        && !text.trim().is_empty()
        && ![
            "<command-name>",
            "<local-command-stdout>",
            "[Request interrupted",
            "<system-reminder>",
        ]
        .iter()
        .any(|prefix| text.starts_with(prefix))
}
