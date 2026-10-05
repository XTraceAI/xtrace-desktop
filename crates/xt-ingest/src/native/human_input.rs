//! Bounded in-memory recognition of Human-only generated input formatting.
//! The ordinary canonical conversion is preserved byte-for-byte.
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use xt_store::{CanonicalRecord, human_input::InputAdjustment, injected::InjectedContextProof};

#[derive(Clone, Default)]
pub(super) struct InputRules {
    questions: BTreeMap<String, Vec<String>>,
    ambiguous: BTreeSet<String>,
    characters: usize,
}
impl InputRules {
    pub fn observe(
        &mut self,
        session: &str,
        record: &CanonicalRecord,
        proof: Option<&InjectedContextProof>,
    ) -> Option<InputAdjustment> {
        let blocks = record.message.content.as_ref()?;
        if record.message.role.as_deref() == Some("assistant") {
            for block in blocks {
                if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                    continue;
                }
                let Some(name) = block.get("name").and_then(Value::as_str) else {
                    continue;
                };
                if !matches!(
                    name,
                    "request_user_input_async" | "functions.request_user_input_async"
                ) {
                    continue;
                }
                let Some(id) = block.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if id.is_empty() || id.len() > 256 {
                    continue;
                }
                let Some(entries) = block
                    .get("input")
                    .and_then(|v| v.get("questions"))
                    .and_then(Value::as_array)
                else {
                    continue;
                };
                if entries.is_empty() || entries.len() > 1024 {
                    continue;
                }
                let Some(questions) = entries
                    .iter()
                    .map(|v| {
                        v.get("title")
                            .or_else(|| v.get("question"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                let size: usize = questions.iter().map(String::len).sum();
                if self.questions.contains_key(id) {
                    self.ambiguous.insert(id.to_owned());
                    continue;
                }
                if self.questions.len() < 1024
                    && self.characters.saturating_add(size) <= 4 * 1024 * 1024
                {
                    self.characters += size;
                    self.questions.insert(id.to_owned(), questions);
                }
            }
            return None;
        }
        if record.message.role.as_deref() != Some("user") {
            return None;
        }
        let texts = blocks
            .iter()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Option<Vec<_>>>()?;
        let text = texts.concat();
        let length = i64::try_from(text.chars().count()).ok()?;
        let (reason, retained) = if proof.is_some() {
            ("selected_skill", None)
        } else if heartbeat(&text) {
            ("heartbeat", None)
        } else if let Some(answers) = self.answer_length(&text) {
            ("question_reply", Some(answers))
        } else {
            return None;
        };
        Some(InputAdjustment {
            record_uuid: record.uuid.clone()?,
            session_id: session.into(),
            original_ts: record.timestamp.clone()?,
            original_length: length,
            retained_length: retained,
            reason: reason.into(),
            native_item_id: proof.map(|p| p.item_id.clone()),
        })
    }
    fn answer_length(&self, text: &str) -> Option<i64> {
        let inner = text
            .trim()
            .strip_prefix("<send_user_message_question_reply>")?
            .strip_suffix("</send_user_message_question_reply>")?;
        #[derive(serde::Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Reply {
            #[serde(rename = "questionItemId")]
            item_id: String,
            question: String,
            answer: String,
        }
        let entries: Vec<Reply> = serde_json::from_str(inner).ok()?;
        if entries.is_empty() || entries.len() > 1024 {
            return None;
        }
        let mut total = 0_i64;
        let mut seen = BTreeSet::new();
        for entry in entries {
            let parts: Vec<Value> = serde_json::from_str(&entry.item_id).ok()?;
            if parts.len() != 3 || parts[0].as_str() != Some("request_user_input_async") {
                return None;
            }
            let call = parts[1].as_str()?;
            let index = usize::try_from(parts[2].as_u64()?).ok()?;
            if self.ambiguous.contains(call) || !seen.insert((call.to_owned(), index)) {
                return None;
            }
            if self.questions.get(call)?.get(index)? != &entry.question {
                return None;
            }
            total = total.checked_add(i64::try_from(entry.answer.chars().count()).ok()?)?;
        }
        Some(total)
    }
}
fn heartbeat(text: &str) -> bool {
    let Some(mut rest) = text
        .trim()
        .strip_prefix("<heartbeat>")
        .and_then(|v| v.strip_suffix("</heartbeat>"))
    else {
        return false;
    };
    let mut fields = Vec::new();
    for name in ["automation_id", "current_time_iso", "instructions"] {
        let open = format!("<{name}>");
        let close = format!("</{name}>");
        let Some(value) = rest.trim_start().strip_prefix(&open) else {
            return false;
        };
        let Some((value, tail)) = value.split_once(&close) else {
            return false;
        };
        if value.trim().is_empty() {
            return false;
        }
        fields.push(value);
        rest = tail;
    }
    rest.trim().is_empty()
        && !fields[0].contains(['<', '>'])
        && chrono::DateTime::parse_from_rfc3339(fields[1].trim()).is_ok()
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn record(role: &str, content: Value) -> CanonicalRecord {
        serde_json::from_value(json!({"uuid":"synthetic-input","type":role,"timestamp":"2026-09-01T00:00:00Z","message":{"role":role,"content":content}})).unwrap()
    }
    #[test]
    fn complete_heartbeat_only_and_not_a_fixed_automation_name() {
        let body = "<heartbeat>\n<automation_id>different-timer</automation_id><current_time_iso>2026-09-01T00:00:00Z</current_time_iso><instructions>check</instructions></heartbeat>";
        assert!(heartbeat(body));
        for bad in [
            format!("quoted {body}"),
            format!("{body} trailing"),
            body.replace("</instructions>", ""),
            body.replace("2026-09-01T00:00:00Z", "bad"),
            "mention <heartbeat>".into(),
        ] {
            assert!(!heartbeat(&bad));
        }
    }
    #[test]
    fn answers_require_the_actual_earlier_question_and_keep_unicode_and_whitespace() {
        let mut rules = InputRules::default();
        let call = record(
            "assistant",
            json!([{"type":"tool_use","name":"request_user_input_async","id":"call-test","input":{"questions":[{"title":"Choose?"}]}}]),
        );
        let make = |question: &str, index: u32| {
            record(
                "user",
                json!([{"type":"text","text":format!("<send_user_message_question_reply>{}</send_user_message_question_reply>",json!([{"questionItemId":json!(["request_user_input_async","call-test",index]).to_string(),"question":question,"answer":" é🙂 "}]))}]),
            )
        };
        assert!(
            rules
                .observe("session", &make("Choose?", 0), None)
                .is_none()
        );
        rules.observe("session", &call, None);
        assert_eq!(
            rules
                .observe("session", &make("Choose?", 0), None)
                .unwrap()
                .retained_length,
            Some(4)
        );
        assert!(rules.observe("session", &make("Other?", 0), None).is_none());
        assert!(
            rules
                .observe("session", &make("Choose?", 1), None)
                .is_none()
        );
    }
}

/// Only the opted-in pinned reader path calls this. Invalid evidence has no
/// effect on the ordinary record and never supplies a fabricated length.
pub(super) fn image_evidence(
    line: &str,
    native: &str,
    session: &str,
    record: &CanonicalRecord,
) -> Option<InputAdjustment> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Evidence {
        contract: String,
        version: u32,
        reason: String,
        record_uuid: String,
        original_ts: String,
        original_length: i64,
        retained_length: i64,
        native_item_id: String,
        native_session_id: String,
    }
    #[derive(serde::Deserialize)]
    struct Envelope {
        human_input_adjustment: Option<Evidence>,
    }
    let proof = serde_json::from_str::<Envelope>(line)
        .ok()?
        .human_input_adjustment?;
    if proof.contract != "xtrace.human_input"
        || proof.version != 1
        || proof.reason != "image_wrapper"
        || proof.native_session_id != native
        || record.uuid.as_deref() != Some(&proof.record_uuid)
        || record.timestamp.as_deref() != Some(&proof.original_ts)
        || record.message.role.as_deref() != Some("user")
        || proof.native_item_id.is_empty()
        || proof.native_item_id.len() > 256
        || !proof
            .native_item_id
            .bytes()
            .all(|b| b.is_ascii_graphic() && !matches!(b, b'/' | b'\\'))
        || proof.retained_length < 0
        || proof.retained_length >= proof.original_length
    {
        return None;
    }
    let length = record
        .message
        .content
        .as_ref()?
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .try_fold(0_i64, |n, b| {
            n.checked_add(i64::try_from(b.get("text")?.as_str()?.chars().count()).ok()?)
        })?;
    if length != proof.original_length {
        return None;
    }
    Some(InputAdjustment {
        record_uuid: proof.record_uuid,
        session_id: session.into(),
        original_ts: proof.original_ts,
        original_length: length,
        retained_length: Some(proof.retained_length),
        reason: proof.reason,
        native_item_id: Some(proof.native_item_id),
    })
}

#[cfg(test)]
mod image_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn image_metadata_must_bind_to_the_same_native_session_record_timestamp_and_length() {
        let record:CanonicalRecord=serde_json::from_value(json!({"uuid":"synthetic","type":"user","timestamp":"2026-09-01T00:00:00Z","message":{"role":"user","content":[{"type":"text","text":"request plus wrapper"}]}})).unwrap();
        let envelope = json!({"human_input_adjustment":{"contract":"xtrace.human_input","version":1,"reason":"image_wrapper","record_uuid":"synthetic","original_ts":"2026-09-01T00:00:00Z","original_length":20,"retained_length":7,"native_item_id":"native:flat:row:1","native_session_id":"native"}});
        let take = |v: &Value| image_evidence(&v.to_string(), "native", "codex-native", &record);
        assert_eq!(take(&envelope).unwrap().retained_length, Some(7));
        for (field, value) in [
            ("version", json!(2)),
            ("record_uuid", json!("other")),
            ("native_session_id", json!("other")),
            ("original_ts", json!("2026-09-02T00:00:00Z")),
            ("original_length", json!(21)),
            ("retained_length", json!(21)),
            ("extra", json!(true)),
        ] {
            let mut bad = envelope.clone();
            bad["human_input_adjustment"][field] = value;
            assert!(take(&bad).is_none());
        }
    }
}
