//! Decoding Codex history rows as data. Every row of a scanned file is
//! decoded for its structural fields, so no escaped spelling of a type or
//! identifier slips past; only the rows a check needs are decoded whole.

use serde::{
    Deserialize, Deserializer,
    de::{IgnoredAny, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use std::fmt;

/// A row's structural fields and nothing else of it.
#[derive(Deserialize)]
pub(super) struct Row {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub ordinal: Option<i64>,
    #[serde(default)]
    pub payload: Option<PayloadHead>,
}

#[derive(Deserialize)]
pub(super) struct PayloadHead {
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// The record's own native identifier, when it is a string. Never a call
    /// identifier.
    #[serde(default)]
    pub id: Option<LooseText>,
    /// A tool output's header, as a code cell writes it.
    #[serde(default)]
    pub output: Option<OutputHead>,
}

/// A value kept only when it is a string.
#[derive(Debug, Default)]
pub(super) struct LooseText(pub Option<String>);

impl<'de> Deserialize<'de> for LooseText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct LooseVisitor;
        impl<'de> Visitor<'de> for LooseVisitor {
            type Value = LooseText;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a value")
            }
            fn visit_str<E>(self, text: &str) -> Result<LooseText, E> {
                Ok(LooseText(Some(text.to_owned())))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<LooseText, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(LooseText(None))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<LooseText, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(LooseText(None))
            }
            fn visit_bool<E>(self, _: bool) -> Result<LooseText, E> {
                Ok(LooseText(None))
            }
            fn visit_i64<E>(self, _: i64) -> Result<LooseText, E> {
                Ok(LooseText(None))
            }
            fn visit_u64<E>(self, _: u64) -> Result<LooseText, E> {
                Ok(LooseText(None))
            }
            fn visit_f64<E>(self, _: f64) -> Result<LooseText, E> {
                Ok(LooseText(None))
            }
            fn visit_unit<E>(self) -> Result<LooseText, E> {
                Ok(LooseText(None))
            }
        }
        deserializer.deserialize_any(LooseVisitor)
    }
}

/// What a code cell's output says first, as its first text item begins.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) enum CellHeader {
    /// Anything else: not a cell's output header.
    #[default]
    Other,
    Completed,
    /// Still running as this cell.
    Running(String),
}

pub(super) const RUNNING: &str = "Script running with cell ID ";
pub(super) const COMPLETED: &str = "Script completed\n";

/// The header a cell output's first item holds.
pub(super) fn cell_header(text: &str) -> CellHeader {
    if let Some(rest) = text.strip_prefix(RUNNING) {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        return if digits == 0 {
            CellHeader::Other
        } else {
            CellHeader::Running(rest[..digits].to_owned())
        };
    }
    if text.starts_with(COMPLETED) {
        CellHeader::Completed
    } else {
        CellHeader::Other
    }
}

/// An output's header, read as [`item`] reads the output — one string, or
/// parts of which only `input_text` ones are text — without keeping the
/// output itself.
#[derive(Debug, Default)]
pub(super) struct OutputHead(pub CellHeader);

struct OutputVisitor;

impl<'de> Visitor<'de> for OutputVisitor {
    type Value = OutputHead;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a tool output")
    }

    fn visit_str<E>(self, text: &str) -> Result<OutputHead, E> {
        Ok(OutputHead(cell_header(text)))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<OutputHead, A::Error> {
        let first = seq.next_element::<PartHead>()?;
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(OutputHead(first.map(|part| part.0).unwrap_or_default()))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<OutputHead, A::Error> {
        while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
        Ok(OutputHead::default())
    }

    fn visit_bool<E>(self, _: bool) -> Result<OutputHead, E> {
        Ok(OutputHead::default())
    }

    fn visit_i64<E>(self, _: i64) -> Result<OutputHead, E> {
        Ok(OutputHead::default())
    }

    fn visit_u64<E>(self, _: u64) -> Result<OutputHead, E> {
        Ok(OutputHead::default())
    }

    fn visit_f64<E>(self, _: f64) -> Result<OutputHead, E> {
        Ok(OutputHead::default())
    }

    fn visit_unit<E>(self) -> Result<OutputHead, E> {
        Ok(OutputHead::default())
    }
}

impl<'de> Deserialize<'de> for OutputHead {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(OutputVisitor)
    }
}

/// One output part's header: its text's, when it is an `input_text` part.
/// A key given twice counts as its last, as a decoded value has it.
struct PartHead(CellHeader);

enum PartKey {
    Type,
    Text,
    Other,
}

impl<'de> Deserialize<'de> for PartKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct KeyVisitor;
        impl Visitor<'_> for KeyVisitor {
            type Value = PartKey;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a key")
            }
            fn visit_str<E>(self, key: &str) -> Result<PartKey, E> {
                Ok(match key {
                    "type" => PartKey::Type,
                    "text" => PartKey::Text,
                    _ => PartKey::Other,
                })
            }
        }
        deserializer.deserialize_str(KeyVisitor)
    }
}

/// Whether a value is the string `input_text`.
struct InputText(bool);

impl<'de> Deserialize<'de> for InputText {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let head = TextHead::deserialize(deserializer)?;
        Ok(Self(head.1))
    }
}

/// A string's cell header, or `None` for any other value; and whether the
/// string is `input_text`.
struct TextHead(Option<CellHeader>, bool);

impl<'de> Deserialize<'de> for TextHead {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct TextVisitor;
        impl<'de> Visitor<'de> for TextVisitor {
            type Value = TextHead;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a value")
            }
            fn visit_str<E>(self, text: &str) -> Result<TextHead, E> {
                Ok(TextHead(Some(cell_header(text)), text == "input_text"))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<TextHead, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(TextHead(None, false))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<TextHead, A::Error> {
                while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
                Ok(TextHead(None, false))
            }
            fn visit_bool<E>(self, _: bool) -> Result<TextHead, E> {
                Ok(TextHead(None, false))
            }
            fn visit_i64<E>(self, _: i64) -> Result<TextHead, E> {
                Ok(TextHead(None, false))
            }
            fn visit_u64<E>(self, _: u64) -> Result<TextHead, E> {
                Ok(TextHead(None, false))
            }
            fn visit_f64<E>(self, _: f64) -> Result<TextHead, E> {
                Ok(TextHead(None, false))
            }
            fn visit_unit<E>(self) -> Result<TextHead, E> {
                Ok(TextHead(None, false))
            }
        }
        deserializer.deserialize_any(TextVisitor)
    }
}

impl<'de> Deserialize<'de> for PartHead {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct PartVisitor;
        impl<'de> Visitor<'de> for PartVisitor {
            type Value = PartHead;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("an output part")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<PartHead, A::Error> {
                let (mut text_part, mut header) = (false, None);
                while let Some(key) = map.next_key::<PartKey>()? {
                    match key {
                        PartKey::Type => text_part = map.next_value::<InputText>()?.0,
                        PartKey::Text => header = map.next_value::<TextHead>()?.0,
                        PartKey::Other => {
                            map.next_value::<IgnoredAny>()?;
                        }
                    }
                }
                // Only an `input_text` part's string is text.
                Ok(PartHead(
                    header.filter(|_| text_part).unwrap_or(CellHeader::Other),
                ))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<PartHead, A::Error> {
                while seq.next_element::<IgnoredAny>()?.is_some() {}
                Ok(PartHead(CellHeader::Other))
            }
            fn visit_str<E>(self, _: &str) -> Result<PartHead, E> {
                Ok(PartHead(CellHeader::Other))
            }
            fn visit_bool<E>(self, _: bool) -> Result<PartHead, E> {
                Ok(PartHead(CellHeader::Other))
            }
            fn visit_i64<E>(self, _: i64) -> Result<PartHead, E> {
                Ok(PartHead(CellHeader::Other))
            }
            fn visit_u64<E>(self, _: u64) -> Result<PartHead, E> {
                Ok(PartHead(CellHeader::Other))
            }
            fn visit_f64<E>(self, _: f64) -> Result<PartHead, E> {
                Ok(PartHead(CellHeader::Other))
            }
            fn visit_unit<E>(self) -> Result<PartHead, E> {
                Ok(PartHead(CellHeader::Other))
            }
        }
        deserializer.deserialize_any(PartVisitor)
    }
}

/// A row's structural fields, or `None` for a line that is not one JSON
/// object with them well typed (a string type, an integer ordinal, no key
/// twice). The caller reserves [`row_bound`] of it first.
pub(super) fn row(line: &[u8]) -> Option<Row> {
    serde_json::from_slice(line).ok()
}

/// A bound on the memory [`row`] takes: the decoder's scratch while it
/// unescapes a string it keeps or looks at (with its growth), and the kept
/// strings, each at most the line. Tested against the actual allocations.
pub(super) fn row_bound(line: &[u8]) -> usize {
    line.len().saturating_mul(3).saturating_add(1024)
}

impl Row {
    pub fn payload_kind(&self) -> Option<&str> {
        self.payload.as_ref()?.kind.as_deref()
    }

    pub fn call_id(&self) -> Option<&str> {
        self.payload.as_ref()?.call_id.as_deref()
    }

    /// An output the Codex app wrote without a call identifier, as the
    /// reviewed reader accepts it: its own native identifier (a non-blank
    /// string) and an output value. It pairs with no call.
    pub fn unpairable_output(&self) -> bool {
        let Some(payload) = self.payload.as_ref() else {
            return false;
        };
        self.is_output()
            && payload.call_id.is_none()
            && payload
                .id
                .as_ref()
                .and_then(|id| id.0.as_deref())
                .is_some_and(|id| !id.trim().is_empty())
            && payload.output.is_some()
    }

    /// Whether the row is a tool call, and whether it is a tool output.
    pub fn is_call(&self) -> bool {
        self.kind.as_deref() == Some("response_item")
            && matches!(
                self.payload_kind(),
                Some("custom_tool_call" | "function_call")
            )
    }

    pub fn is_output(&self) -> bool {
        self.kind.as_deref() == Some("response_item")
            && matches!(
                self.payload_kind(),
                Some("custom_tool_call_output" | "function_call_output")
            )
    }

    /// Whether the row is a code-mode `exec` call, the only kind a launch is.
    pub fn is_exec(&self) -> bool {
        self.kind.as_deref() == Some("response_item")
            && self.payload_kind() == Some("custom_tool_call")
            && self.payload.as_ref().and_then(|p| p.name.as_deref()) == Some("exec")
    }
}

/// Parse an RFC 3339 timestamp to UTC milliseconds.
pub(super) fn millis(text: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|at| at.timestamp_millis())
}

/// A decoded row's own time.
pub(super) fn timestamp(value: &Value) -> Option<i64> {
    value
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(millis)
}

/// Stands in for an output item that is not text: no reader accepts it.
pub(super) const NON_TEXT: &str = "\u{0}non-text";

/// The fixed completed-cell layout for `store(key,id);text({session_id:id})`:
/// header, optional literal patch result, UUID receipt, then exactly the
/// parsed tool operation results. `store` itself emits no result.
pub(super) fn binding_receipt(
    items: &[String],
    prefix: usize,
    operations: usize,
    literal: Option<&str>,
) -> Option<String> {
    if prefix > 1
        || items.len() != operations.checked_add(prefix)?.checked_add(2)?
        || cell_header(items.first()?) != CellHeader::Completed
        || (prefix == 1 && items.get(1)?.as_str() != "{}")
    {
        return None;
    }
    let value: Value = serde_json::from_str(items.get(1 + prefix)?).ok()?;
    let object = value.as_object()?;
    if object.len() != 1 {
        return None;
    }
    let id = object.get("session_id")?.as_str()?;
    (items.get(1 + prefix)? == &format!("{{\"session_id\":\"{id}\"}}")
        && super::command::uuid(id)
        && literal.is_none_or(|literal| literal == id))
    .then(|| id.to_owned())
}

#[cfg(test)]
mod binding_receipt_tests {
    use super::*;
    #[test]
    fn only_the_fixed_own_output_slot_is_a_receipt() {
        let id = "7fd904e4-b007-4f55-a1d1-40066c96457b";
        let items = vec![
            "Script completed\nOutput:\n".into(),
            "{}".into(),
            format!("{{\"session_id\":\"{id}\"}}"),
            "git result".into(),
        ];
        assert_eq!(binding_receipt(&items, 1, 1, None).as_deref(), Some(id));
        assert!(binding_receipt(&items, 0, 1, None).is_none());
        assert!(
            binding_receipt(&items, 1, 1, Some("00000000-0000-4000-8000-000000000000")).is_none()
        );
        let mut wrong = items;
        wrong[2] = "{}".into();
        wrong[3] = format!("{{\"session_id\":\"{id}\"}}");
        assert!(binding_receipt(&wrong, 1, 1, None).is_none());
    }
}

/// One tool call or tool output row, whole.
#[derive(Clone, Debug)]
pub(super) enum Item {
    Call {
        call_id: String,
        name: String,
        custom: bool,
        input: String,
    },
    Output {
        call_id: String,
        /// Text items in order; a plain string output is one item.
        items: Vec<String>,
    },
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key)?.as_str().map(str::to_owned)
}

/// A decoded row as an item, if it is a tool call or tool output; `Err` for
/// one whose required fields are missing.
pub(super) fn item(value: &Value) -> Result<Option<Item>, ()> {
    if value.get("type").and_then(Value::as_str) != Some("response_item") {
        return Ok(None);
    }
    let payload = &value["payload"];
    let kind = payload.get("type").and_then(Value::as_str).unwrap_or("");
    Ok(match kind {
        "custom_tool_call" | "function_call" => Some(Item::Call {
            call_id: text(payload, "call_id").ok_or(())?,
            name: text(payload, "name").ok_or(())?,
            custom: kind == "custom_tool_call",
            input: text(payload, "input")
                .or_else(|| text(payload, "arguments"))
                .ok_or(())?,
        }),
        "custom_tool_call_output" | "function_call_output" => {
            let items = match payload.get("output") {
                Some(Value::String(one)) => vec![one.clone()],
                Some(Value::Array(parts)) => parts
                    .iter()
                    .map(|part| {
                        (part.get("type").and_then(Value::as_str) == Some("input_text"))
                            .then(|| text(part, "text"))
                            .flatten()
                            .unwrap_or_else(|| NON_TEXT.to_owned())
                    })
                    .collect(),
                _ => vec![NON_TEXT.to_owned()],
            };
            Some(Item::Output {
                call_id: text(payload, "call_id").ok_or(())?,
                items,
            })
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_decoded_for_its_structure_whatever_its_spelling() {
        let meta = row(br#"{"type":"session_meta","ordinal":0,"payload":{"id":"x"}}"#).unwrap();
        assert_eq!(meta.kind.as_deref(), Some("session_meta"));
        assert_eq!(meta.ordinal, Some(0));
        // An escaped spelling decodes to the same type.
        let escaped = row(br#"{"type":"session_meta","payload":{}}"#).unwrap();
        assert_eq!(escaped.kind.as_deref(), Some("session_meta"));
        let call = row(
            br#"{"type":"response_item","ordinal":7,"payload":{"type":"custom_tool_call","call_id":"c","name":"exec","input":"x"}}"#,
        )
        .unwrap();
        assert!(call.is_call() && call.is_exec() && !call.is_output());
        assert_eq!(call.call_id(), Some("c"));
        for bad in [
            &br#"[1]"#[..],
            br#"{"type":1}"#,
            br#"{"type":"x","ordinal":"7"}"#,
            br#"{"type":"x","ordinal":7.5}"#,
            br#"{"type":"x","type":"y"}"#,
            br#"{"type":"x","payload":{"call_id":3}}"#,
            br#"{"type":"x""#,
        ] {
            assert!(row(bad).is_none(), "{}", String::from_utf8_lossy(bad));
        }
    }
}
