//! The transcript payload: one session's own records, typed for the view that
//! renders them.
//!
//! This is the half of the session-source contract that was deliberately left
//! out until the components that draw a transcript existed. It exists now, and
//! it is a **translation, not a passthrough**. A canonical record is a native
//! line with a native shape; what crosses here is the record's identity, who
//! it is from, when it was recorded, and its content blocks in the order the
//! record states them — each block mapped to a closed variant the view knows
//! how to draw.
//!
//! Three rules, and every field below follows from one of them:
//!
//! - **Identity is carried, never re-derived.** A record is its own UUID and a
//!   block is `<uuid>:<index>`, which is the identity the index already gives a
//!   tool call (`tool_uses.uuid`, `tool_uses.block_index`). Nothing here
//!   renumbers, normalises or invents one, and no position in a list is treated
//!   as an ordinal.
//! - **A block this contract does not know is named, not shipped.** The
//!   fallback is [`SourceBlock::Unsupported`] carrying the block's own stated
//!   type, bounded to a label; it is never the raw JSON of the block. That
//!   keeps images, audio, documents and whatever ships next out of the view
//!   entirely: there is no field here that could carry a URL or a base64 body
//!   for something to load.
//! - **Words are the record's, wording is the view's.** Text crosses verbatim,
//!   because it is the transcript. Everything else is a closed variant or a
//!   bounded label, so no sentence is written here for the view to display.
//!
//! The one payload that is genuinely open is a tool call's input, which is
//! whatever the tool was given. The view's contract takes it as an opaque
//! value it prints and never interprets, so it crosses as one.
use serde::Serialize;
use serde_json::Value;
use ts_rs::TS;
use xt_ingest::canonical::ParsedRecord;
use xt_store::model::RecordType;

/// The most characters a block's own type may contribute to its label. A type
/// is a short word in every shape anyone writes; a longer one is not a label
/// and is dropped rather than printed.
const MAX_LABEL: usize = 64;

/// Who a record is from, as the record itself states it.
///
/// Two values, because the canonical record has two. What a turn *was* —
/// a person typing, a system reminder, a tool returning — is classification,
/// which M-02 owns and this transport does not restate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RecordRole {
    User,
    Assistant,
}

/// What a tool was given. `text` is a payload the record stated as text;
/// `json` is the call's own arguments, carried as the value they are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourcePayload {
    Text {
        text: String,
    },
    Json {
        #[ts(type = "unknown")]
        value: Value,
    },
}

/// One part of a tool's result.
///
/// A result may be a string, or a list of parts of which only some are text.
/// A part that is not text is **named and not carried**: a screenshot returned
/// by a tool is a part of the record this view will not show, and its bytes
/// have no business crossing a contract that promises to load nothing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceResultPart {
    Text { text: String },
    Unshown { label: Option<String> },
}

/// One content block of a record, in the record's own order.
///
/// **`index` is the block's real place in its record's content**, which is the
/// half of a block's structural identity that is not the record's UUID. The
/// index the store keeps beside a measured tool call (`tool_uses.block_index`)
/// is this number, so `{record uuid, block index}` names the same block on
/// both sides and a later timeline can jump to a measured call without a
/// second identity scheme. Nothing is composed here: a single opaque string
/// would have to be taken apart again to be useful, and the part that matters
/// is a number.
///
/// `call_id` is separate from both. It is the **host's own** identifier for a
/// call (`toolu_…`), which is what a result names to say which call it
/// answers; the store's `tool_uses.id` is a local row number and is not it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SourceBlock {
    Text {
        index: u32,
        text: String,
    },
    /// Extended thinking, as the record recorded it.
    Thinking {
        index: u32,
        text: String,
    },
    /// A call the assistant made. `call_id` is the record's own identifier for
    /// it, which a result names to say which call it answers.
    ToolCall {
        index: u32,
        /// Verbatim, and an open set: an MCP server names its own tools.
        name: String,
        call_id: Option<String>,
        input: Option<SourcePayload>,
    },
    /// A result, where the record put it. `failed` is the record's own
    /// `is_error`; absent means it did not say so, which is not a failure.
    ToolResult {
        index: u32,
        call_id: Option<String>,
        failed: bool,
        parts: Vec<SourceResultPart>,
    },
    /// A block this contract does not translate, named by its own stated type.
    Unsupported {
        index: u32,
        label: Option<String>,
    },
}

impl SourceBlock {
    /// Where this block sits in its record's content.
    pub fn index(&self) -> u32 {
        match *self {
            Self::Text { index, .. }
            | Self::Thinking { index, .. }
            | Self::ToolCall { index, .. }
            | Self::ToolResult { index, .. }
            | Self::Unsupported { index, .. } => index,
        }
    }
}

/// One record of a session: its identity, who it is from, when it was
/// recorded, and what it carried.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct SourceRecord {
    /// The record's own UUID, exactly. With a block's `index` this is the
    /// structural locator a measurement already carries, so a later timeline
    /// can name a block without anything here being parsed or re-derived.
    ///
    /// It is as unique as the history made it: a fork copies a record's
    /// prefix, so one session can carry two records under one UUID. Both are
    /// reported. A caller that resolves a locator to one block must require
    /// the match to be unique rather than take the first.
    pub id: String,
    pub role: RecordRole,
    /// The instant the record states, verbatim (RFC 3339, validated by the
    /// canonical parser). Which zone to read it in is the view's decision, as
    /// it is on every other screen, so no label is worded here.
    pub at: Option<String>,
    /// In record order. An absent content field is no blocks — which is not
    /// the same as a record that carried an empty list, and neither is
    /// reported as content that was shown.
    pub blocks: Vec<SourceBlock>,
}

impl TryFrom<&ParsedRecord> for SourceRecord {
    type Error = &'static str;

    fn try_from(record: &ParsedRecord) -> Result<Self, Self::Error> {
        // The canonical parser drops a user or assistant line with no usable
        // UUID before it becomes a record, so this cannot be reached from a
        // loaded session. It is a refusal rather than a fabricated identity
        // because an invented id would make two records look like one.
        //
        // **Trimming decides whether an identity is usable; it never decides
        // what the identity is.** The store keys a record by the bytes the
        // record carried, and a later jump from a measurement to a block has
        // to compare against those bytes. Two identities that differ only in
        // whitespace are two identities here as they are everywhere else, so
        // what crosses is the recorded value, untouched.
        let id = record
            .canonical
            .uuid
            .as_deref()
            .filter(|uuid| !uuid.trim().is_empty())
            .ok_or("transcript record has no identity")?;
        Ok(Self {
            id: id.to_owned(),
            role: match record.canonical.record_type {
                RecordType::User => RecordRole::User,
                RecordType::Assistant => RecordRole::Assistant,
            },
            at: record.canonical.timestamp.clone(),
            blocks: record
                .canonical
                .message
                .content
                .as_deref()
                .unwrap_or_default()
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    let index = u32::try_from(index)
                        .map_err(|_| "transcript record has more blocks than can be indexed")?;
                    Ok(block(index, value))
                })
                .collect::<Result<_, Self::Error>>()?,
        })
    }
}

/// A short, printable name for a block type or a result part type, or nothing.
///
/// The value comes off the record, so it is bounded to a label shape before it
/// can be shown: a type is a word, and anything longer or stranger than a word
/// is a value being used as a sentence. Dropping it leaves the view saying a
/// block is not shown, which is the true half of the statement either way.
fn label(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    let shaped = !value.is_empty()
        && value.chars().count() <= MAX_LABEL
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    shaped.then(|| value.to_owned())
}

fn text_of(block: &Value, key: &str) -> Option<String> {
    block.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// A tool call's arguments as the record carried them.
fn payload(value: &Value) -> SourcePayload {
    match value {
        Value::String(text) => SourcePayload::Text { text: text.clone() },
        other => SourcePayload::Json {
            value: other.clone(),
        },
    }
}

/// One part of a tool result: its text, or the name of what is not shown.
fn result_part(value: &Value) -> SourceResultPart {
    match value.get("type").and_then(Value::as_str) {
        Some("text") => match text_of(value, "text") {
            Some(text) => SourceResultPart::Text { text },
            // A part that calls itself text and carries none is not text.
            None => SourceResultPart::Unshown {
                label: label(Some("text")),
            },
        },
        other => SourceResultPart::Unshown {
            label: label(other),
        },
    }
}

/// A result's `content`, which the host writes as a string, as a list of
/// parts, or not at all.
fn result_parts(block: &Value) -> Vec<SourceResultPart> {
    match block.get("content") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(text)) => vec![SourceResultPart::Text { text: text.clone() }],
        Some(Value::Array(parts)) => parts.iter().map(result_part).collect(),
        Some(other) => vec![result_part(other)],
    }
}

fn block(index: u32, value: &Value) -> SourceBlock {
    // Every block of a parsed record is an object carrying a string `type`;
    // the canonical parser refuses the line otherwise. An unknown type is
    // expected and ordinary — the block set is the host's, and it grows.
    let kind = value.get("type").and_then(Value::as_str);
    match kind {
        // A text block's own text is guaranteed by the canonical parser.
        Some("text") => match text_of(value, "text") {
            Some(text) => SourceBlock::Text { index, text },
            None => SourceBlock::Unsupported {
                index,
                label: label(kind),
            },
        },
        Some("thinking") => match text_of(value, "thinking") {
            Some(text) => SourceBlock::Thinking { index, text },
            // Redacted thinking carries no readable text, and neither does a
            // thinking block that states none. Named, never guessed at.
            None => SourceBlock::Unsupported {
                index,
                label: label(kind),
            },
        },
        // A tool call's name is non-blank by the canonical parser's contract.
        Some("tool_use") => SourceBlock::ToolCall {
            index,
            name: text_of(value, "name").unwrap_or_default(),
            call_id: text_of(value, "id"),
            input: value.get("input").map(payload),
        },
        Some("tool_result") => SourceBlock::ToolResult {
            index,
            call_id: text_of(value, "tool_use_id"),
            failed: value.get("is_error").and_then(Value::as_bool) == Some(true),
            parts: result_parts(value),
        },
        _ => SourceBlock::Unsupported {
            index,
            label: label(kind),
        },
    }
}

/// Every record of a loaded session, in the order it was read.
pub fn records(records: &[ParsedRecord]) -> Result<Vec<SourceRecord>, &'static str> {
    records.iter().map(SourceRecord::try_from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xt_ingest::canonical::{Parsed, parse_line};

    /// One canonical line, parsed exactly as the loader parses one, so these
    /// cases are judged against the parser's real guarantees rather than a
    /// record built by hand around them.
    fn record(line: Value) -> ParsedRecord {
        match parse_line(&line.to_string()).unwrap() {
            Parsed::Record(record) => *record,
            other => panic!("{other:?}"),
        }
    }

    fn assistant(content: Value) -> SourceRecord {
        SourceRecord::try_from(&record(json!({
            "uuid": "11111111-1111-4111-8111-111111111111",
            "type": "assistant",
            "timestamp": "2026-09-07T12:00:00.000Z",
            "message": {"role": "assistant", "content": content}
        })))
        .unwrap()
    }

    fn user(content: Value) -> SourceRecord {
        SourceRecord::try_from(&record(json!({
            "uuid": "22222222-2222-4222-8222-222222222222",
            "type": "user",
            "message": {"role": "user", "content": content}
        })))
        .unwrap()
    }

    #[test]
    fn a_block_is_located_by_its_record_and_its_place_in_it() {
        // The pair the index already stores for a measured tool call
        // (`tool_uses.uuid`, `tool_uses.block_index`). It stays a pair: a
        // number that is really the block's place, beside the record's real
        // UUID, so the locator a measurement carries needs no translation and
        // nothing downstream has to take a composed string apart.
        let record = assistant(json!([
            {"type": "text", "text": "first"},
            {"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {"file_path": "/a"}},
            {"type": "text", "text": "second"},
        ]));
        assert_eq!(
            record
                .blocks
                .iter()
                .map(SourceBlock::index)
                .collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(record.id, "11111111-1111-4111-8111-111111111111");
        assert_eq!(record.role, RecordRole::Assistant);
        assert_eq!(record.at.as_deref(), Some("2026-09-07T12:00:00.000Z"));
        // The call's own identifier is the host's, kept beside the locator and
        // never confused with it: a result names this, a timeline names those.
        let SourceBlock::ToolCall { call_id, .. } = &record.blocks[1] else {
            panic!("{:?}", record.blocks[1]);
        };
        assert_eq!(call_id.as_deref(), Some("toolu_1"));
    }

    #[test]
    fn a_record_with_no_recorded_time_borrows_none() {
        assert_eq!(user(json!([{"type": "text", "text": "hello"}])).at, None);
    }

    #[test]
    fn content_the_record_did_not_carry_is_no_blocks_at_all() {
        // Absent content and an explicitly empty list are different facts
        // about the record, and neither is content that was shown.
        let absent = SourceRecord::try_from(&record(json!({
            "uuid": "33333333-3333-4333-8333-333333333333",
            "type": "assistant", "message": {"role": "assistant"}
        })))
        .unwrap();
        assert!(absent.blocks.is_empty());
        assert!(assistant(json!([])).blocks.is_empty());
    }

    #[test]
    fn a_tool_call_carries_its_name_verbatim_and_its_arguments_as_they_are() {
        let record = assistant(json!([
            {"type": "tool_use", "id": "toolu_9", "name": "mcp__server__do_thing",
             "input": {"command": "ls -al", "flags": [1, 2]}},
        ]));
        assert_eq!(
            record.blocks,
            [SourceBlock::ToolCall {
                index: 0,
                name: "mcp__server__do_thing".into(),
                call_id: Some("toolu_9".into()),
                input: Some(SourcePayload::Json {
                    value: json!({"command": "ls -al", "flags": [1, 2]}),
                }),
            }]
        );
    }

    #[test]
    fn a_tool_name_that_names_a_language_builtin_is_still_just_a_name() {
        // The tool set is open, and a name is data. Nothing here treats one as
        // a key into anything; the view is where this used to be a crash.
        for name in ["__proto__", "constructor", "toString", "hasOwnProperty"] {
            let record = assistant(json!([{"type": "tool_use", "id": "t", "name": name}]));
            let SourceBlock::ToolCall { name: carried, .. } = &record.blocks[0] else {
                panic!("{:?}", record.blocks[0]);
            };
            assert_eq!(carried, name);
        }
    }

    #[test]
    fn a_result_says_which_call_it_answers_and_whether_it_failed() {
        let record = user(json!([
            {"type": "tool_result", "tool_use_id": "toolu_9", "is_error": true,
             "content": "no such file"},
        ]));
        assert_eq!(
            record.blocks,
            [SourceBlock::ToolResult {
                index: 0,
                call_id: Some("toolu_9".into()),
                failed: true,
                parts: vec![SourceResultPart::Text {
                    text: "no such file".into()
                }],
            }]
        );
    }

    #[test]
    fn a_result_that_did_not_say_it_failed_did_not_fail() {
        for content in [json!(null), json!("ok"), json!([])] {
            let record = user(json!([{"type": "tool_result", "tool_use_id": "t",
                                      "content": content}]));
            let SourceBlock::ToolResult { failed, .. } = &record.blocks[0] else {
                panic!("{:?}", record.blocks[0]);
            };
            assert!(!failed);
        }
    }

    #[test]
    fn a_result_part_that_is_not_text_is_named_and_never_carried() {
        // The whole point of the part list: a screenshot a tool returned is a
        // part of the record this app will not show, and its bytes have no
        // business crossing a contract that promises to load nothing.
        let record = user(json!([
            {"type": "tool_result", "tool_use_id": "t", "content": [
                {"type": "text", "text": "before"},
                {"type": "image", "source": {"type": "base64", "media_type": "image/png",
                                             "data": "SECRETBYTES"}},
                {"type": "text", "text": "after"},
            ]},
        ]));
        let SourceBlock::ToolResult { parts, .. } = &record.blocks[0] else {
            panic!("{:?}", record.blocks[0]);
        };
        assert_eq!(
            parts,
            &[
                SourceResultPart::Text {
                    text: "before".into()
                },
                SourceResultPart::Unshown {
                    label: Some("image".into())
                },
                SourceResultPart::Text {
                    text: "after".into()
                },
            ]
        );
        let wire = serde_json::to_string(&record).unwrap();
        assert!(!wire.contains("SECRETBYTES"), "{wire}");
        assert!(!wire.contains("base64"), "{wire}");
    }

    #[test]
    fn a_block_this_contract_does_not_draw_is_named_by_its_own_type() {
        let record = assistant(json!([
            {"type": "image", "source": {"data": "SECRETBYTES"}},
            {"type": "redacted_thinking", "data": "OPAQUE"},
            {"type": "server_tool_use", "id": "s", "name": "web_search"},
        ]));
        assert_eq!(
            record.blocks,
            [
                SourceBlock::Unsupported {
                    index: 0,
                    label: Some("image".into()),
                },
                SourceBlock::Unsupported {
                    index: 1,
                    label: Some("redacted_thinking".into()),
                },
                SourceBlock::Unsupported {
                    index: 2,
                    label: Some("server_tool_use".into()),
                },
            ]
        );
        let wire = serde_json::to_string(&record).unwrap();
        for absent in ["SECRETBYTES", "OPAQUE", "web_search"] {
            assert!(!wire.contains(absent), "{wire}");
        }
    }

    #[test]
    fn a_type_that_is_a_sentence_rather_than_a_label_is_dropped() {
        // The value comes off the record, and the view prints it. A label is a
        // word; anything else is a value being used as a sentence, and the
        // block is still named as one this view does not draw.
        for kind in [
            json!("a type with spaces"),
            json!("<script>alert(1)</script>"),
            json!("x".repeat(MAX_LABEL + 1)),
            json!("  "),
        ] {
            let record = assistant(json!([{"type": kind}]));
            assert_eq!(
                record.blocks,
                [SourceBlock::Unsupported {
                    index: 0,
                    label: None,
                }],
                "{kind}"
            );
        }
        // A label exactly at the bound is still a label.
        let kind = "x".repeat(MAX_LABEL);
        let record = assistant(json!([{"type": kind}]));
        assert_eq!(
            record.blocks,
            [SourceBlock::Unsupported {
                index: 0,
                label: Some(kind),
            }]
        );
    }

    #[test]
    fn thinking_crosses_as_thinking_and_redacted_thinking_does_not() {
        let record = assistant(json!([
            {"type": "thinking", "thinking": "weighing it up", "signature": "sig"},
            {"type": "thinking", "signature": "sig"},
        ]));
        assert_eq!(
            record.blocks,
            [
                SourceBlock::Thinking {
                    index: 0,
                    text: "weighing it up".into(),
                },
                // It called itself thinking and carried none. Named, not guessed.
                SourceBlock::Unsupported {
                    index: 1,
                    label: Some("thinking".into()),
                },
            ]
        );
        assert!(!serde_json::to_string(&record).unwrap().contains("sig"));
    }

    #[test]
    fn text_crosses_exactly_as_the_record_stated_it() {
        // Markup, control characters and an unpaired-looking surrogate escape
        // are the transcript's own words. Nothing is escaped, stripped or
        // rewritten here; what the view does with them is the view's contract.
        let odd = "<script>x</script>\t\u{0007}\u{200b}emoji 🙂 \\u0041 ünïcödé";
        let record = user(json!([{"type": "text", "text": odd}]));
        assert_eq!(
            record.blocks,
            [SourceBlock::Text {
                index: 0,
                text: odd.into(),
            }]
        );
    }

    #[test]
    fn a_string_message_is_the_one_text_block_the_parser_made_of_it() {
        let record = SourceRecord::try_from(&record(json!({
            "uuid": "44444444-4444-4444-8444-444444444444",
            "type": "user", "message": {"role": "user", "content": "just text"}
        })))
        .unwrap();
        assert_eq!(
            record.blocks,
            [SourceBlock::Text {
                index: 0,
                text: "just text".into(),
            }]
        );
    }

    #[test]
    fn a_user_record_is_from_the_user_and_an_assistant_record_from_the_assistant() {
        // Two values because the record has two. What a turn *was* is
        // classification, which M-02 owns; this says only what the record says.
        assert_eq!(user(json!([])).role, RecordRole::User);
        assert_eq!(assistant(json!([])).role, RecordRole::Assistant);
    }

    #[test]
    fn a_record_identity_crosses_as_the_bytes_it_was_recorded_with() {
        // The store keys a record by the bytes the record carried, and the
        // parser only trims to decide whether an identity is usable. Trimming
        // here as well would make two distinct records one, and would break a
        // later jump that compares a measurement's identity to a block's.
        for recorded in [
            " 11111111-1111-4111-8111-111111111111",
            "11111111-1111-4111-8111-111111111111 ",
            "\t11111111-1111-4111-8111-111111111111\n",
        ] {
            let record = record(json!({
                "uuid": recorded, "type": "user",
                "message": {"role": "user", "content": [{"type": "text", "text": "hi"}]}
            }));
            assert_eq!(record.canonical.uuid.as_deref(), Some(recorded));
            assert_eq!(SourceRecord::try_from(&record).unwrap().id, recorded);
        }
    }

    #[test]
    fn identities_that_differ_only_in_whitespace_stay_different() {
        let bare = "11111111-1111-4111-8111-111111111111";
        let padded = format!(" {bare}");
        let one = SourceRecord::try_from(&record(json!({
            "uuid": bare, "type": "user", "message": {"role": "user", "content": []}
        })))
        .unwrap();
        let other = SourceRecord::try_from(&record(json!({
            "uuid": padded, "type": "user", "message": {"role": "user", "content": []}
        })))
        .unwrap();
        assert_ne!(one.id, other.id);
        assert_eq!(other.id, padded);
    }

    #[test]
    fn a_record_with_no_identity_is_refused_rather_than_given_one() {
        let mut record = record(json!({
            "uuid": "55555555-5555-4555-8555-555555555555",
            "type": "user", "message": {"role": "user", "content": []}
        }));
        for empty in [None, Some(String::new()), Some("   ".into())] {
            record.canonical.uuid = empty;
            assert!(SourceRecord::try_from(&record).is_err());
        }
    }

    #[test]
    fn a_tool_call_given_a_string_carries_it_as_text() {
        let record = assistant(json!([
            {"type": "tool_use", "id": "t", "name": "Bash", "input": "ls"},
        ]));
        let SourceBlock::ToolCall { input, .. } = &record.blocks[0] else {
            panic!("{:?}", record.blocks[0]);
        };
        assert_eq!(
            input,
            &Some(SourcePayload::Text {
                text: "ls".to_owned()
            })
        );
    }
}
