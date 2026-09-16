//! Pure native-line classification into the shared canonical record protocol.
//! No persistent writer, native database reader or URL-link authority lives here.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fmt;
use xt_store::{CanonicalRecord, SessionSource};

/// Observed import/header metadata. Keeping this separate preserves conflicting
/// native and header identities for the writer instead of silently choosing one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceContext {
    pub conversation_id: Option<String>,
    pub native_session_id: Option<String>,
    pub source_platform: Option<String>,
    pub source_surface: Option<String>,
    pub started_at: Option<String>,
    pub source: Option<SessionSource>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeMetadata {
    /// Set only by the native adapter after validating explicit iteration usage.
    #[serde(skip)]
    pub iteration_usage_confirmed: bool,
    /// Native `sessionId`, independent of the canonical header conversation ID.
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub parent_uuid: Option<String>,
    pub subtype: Option<String>,
    pub entrypoint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParsedRecord {
    pub canonical: CanonicalRecord,
    pub native: NativeMetadata,
    pub source: SourceContext,
    pub context: SourceContext,
    /// None means content was absent; zero means a measured array/string.
    pub tool_use_count: Option<usize>,
    pub is_tool_result_carrier: Option<bool>,
}

/// One native summary event, not one event per hook invocation. Commands,
/// errors, additional context and stop-reason text are deliberately not retained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopHookSummary {
    pub uuid: Option<String>,
    pub timestamp: Option<String>,
    pub native: NativeMetadata,
    pub source: SourceContext,
    pub context: SourceContext,
    pub hook_count: Option<u64>,
    pub prevented_continuation: Option<bool>,
    pub has_output: Option<bool>,
    pub total_duration_ms: Option<u64>,
    pub tool_use_id: Option<String>,
}

/// A native observation. URL normalization, ownership and exact-link validation
/// are later responsibilities; parsing this does not create or authorize a link.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrLink {
    pub native: NativeMetadata,
    pub source: SourceContext,
    pub context: SourceContext,
    pub timestamp: Option<String>,
    pub number: u64,
    pub raw_url: String,
    pub raw_repository: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropReason {
    MissingUuid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Parsed {
    Record(Box<ParsedRecord>),
    StructuralEvent(Box<StopHookSummary>),
    PrLink(Box<PrLink>),
    Inert,
    Dropped(DropReason),
}

/// Fixed error categories never echo a transcript, JSON fragment, identifier or
/// arbitrary field value. The caller may attach its own bounded source location.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParseError {
    InvalidJson,
    ExpectedObject,
    InvalidField,
    InvalidTimestamp,
    InvalidRecord,
    InvalidContent,
    InvalidUsage,
    InvalidPrLink,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidJson => "invalid JSON",
            Self::ExpectedObject => "native line must be an object",
            Self::InvalidField => "invalid structural field type",
            Self::InvalidTimestamp => "native timestamp must be RFC3339",
            Self::InvalidRecord => "invalid canonical record shape",
            Self::InvalidContent => "invalid canonical content block",
            Self::InvalidUsage => "usage counters must be nonnegative integers",
            Self::InvalidPrLink => "native PR link requires a positive number, URL and repository",
        })
    }
}
impl std::error::Error for ParseError {}

pub fn parse_line(line: &str) -> Result<Parsed, ParseError> {
    parse_with_context(line, &SourceContext::default())
}

pub fn parse_with_context(line: &str, context: &SourceContext) -> Result<Parsed, ParseError> {
    let mut value: Value = serde_json::from_str(line).map_err(|_| ParseError::InvalidJson)?;
    let object = value.as_object_mut().ok_or(ParseError::ExpectedObject)?;
    let kind = optional_string(object, "type")?;
    let recognized = matches!(kind.as_deref(), Some("user" | "assistant" | "pr-link"))
        || (kind.as_deref() == Some("system")
            && object.get("subtype").and_then(Value::as_str) == Some("stop_hook_summary"));
    if !recognized {
        return Ok(Parsed::Inert);
    }
    if matches!(kind.as_deref(), Some("user" | "assistant"))
        && optional_string(object, "uuid")?.is_none_or(|id| id.trim().is_empty())
    {
        return Ok(Parsed::Dropped(DropReason::MissingUuid));
    }
    validate_time(context.started_at.as_deref())?;
    let native = NativeMetadata {
        iteration_usage_confirmed: false,
        session_id: optional_string(object, "sessionId")?,
        agent_id: optional_string(object, "agentId")?,
        parent_uuid: optional_string(object, "parentUuid")?,
        subtype: optional_string(object, "subtype")?,
        entrypoint: optional_string(object, "entrypoint")?,
    };
    let timestamp = optional_string(object, "timestamp")?;
    validate_time(timestamp.as_deref())?;
    let source = line_source(object)?;
    if kind.as_deref() == Some("pr-link") {
        let number = optional_u64(object, "prNumber")?
            .filter(|number| *number > 0)
            .ok_or(ParseError::InvalidPrLink)?;
        return Ok(Parsed::PrLink(Box::new(PrLink {
            native,
            source,
            context: context.clone(),
            timestamp,
            number,
            raw_url: required_string(object, "prUrl")?,
            raw_repository: required_string(object, "prRepository")?,
        })));
    }
    if kind.as_deref() == Some("system") {
        return Ok(Parsed::StructuralEvent(Box::new(StopHookSummary {
            uuid: optional_string(object, "uuid")?,
            timestamp,
            native,
            source,
            context: context.clone(),
            hook_count: optional_u64(object, "hookCount")?,
            prevented_continuation: optional_bool(object, "preventedContinuation")?,
            has_output: optional_bool(object, "hasOutput")?,
            total_duration_ms: optional_u64(object, "totalDurationMs")?,
            tool_use_id: optional_string(object, "toolUseID")?,
        })));
    }
    // Native user content can be a string; the shared protocol represents it as
    // one text block. No missing content or usage is turned into a measured zero.
    if object.get("message").is_none_or(Value::is_null) {
        object.insert("message".into(), Value::Object(Map::new()));
    }
    let message = object
        .get_mut("message")
        .and_then(Value::as_object_mut)
        .ok_or(ParseError::InvalidRecord)?;
    if let Some(Value::String(text)) = message.get("content") {
        message.insert(
            "content".into(),
            serde_json::json!([{"type":"text","text":text}]),
        );
    }
    let mut canonical: CanonicalRecord =
        serde_json::from_value(value).map_err(|_| ParseError::InvalidRecord)?;
    if canonical.api_message_id.is_none() {
        canonical.api_message_id = canonical.message.id.clone();
    }
    if canonical.source_surface.is_none() {
        canonical.source_surface = native.entrypoint.clone();
    }
    if canonical.native_session_id.is_none() {
        canonical.native_session_id = native.session_id.clone();
    }
    // A blank cwd or branch is no label (the producers map "" to null for
    // theirs): it must never fill a session's fill-once metadata ahead of a
    // real value, which could then not replace it.
    for label in [&mut canonical.cwd, &mut canonical.git_branch] {
        if label
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            *label = None;
        }
    }
    let (tool_use_count, is_tool_result_carrier) = inspect_content(&canonical)?;
    if let Some(usage) = &canonical.message.usage {
        let cache = usage.cache_creation.as_ref();
        if [
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_input_tokens,
            usage.cache_creation_input_tokens,
            cache.and_then(|c| c.ephemeral_5m_input_tokens),
            cache.and_then(|c| c.ephemeral_1h_input_tokens),
        ]
        .into_iter()
        .flatten()
        .any(|count| count < 0)
        {
            return Err(ParseError::InvalidUsage);
        }
    }
    Ok(Parsed::Record(Box::new(ParsedRecord {
        canonical,
        native,
        source,
        context: context.clone(),
        tool_use_count,
        is_tool_result_carrier,
    })))
}

fn line_source(object: &Map<String, Value>) -> Result<SourceContext, ParseError> {
    let started_at = optional_string(object, "started_at")?;
    validate_time(started_at.as_deref())?;
    let source = match object.get("source") {
        None | Some(Value::Null) => None,
        Some(value) => {
            Some(serde_json::from_value(value.clone()).map_err(|_| ParseError::InvalidField)?)
        }
    };
    Ok(SourceContext {
        conversation_id: optional_string(object, "conversation_id")?,
        native_session_id: optional_string(object, "native_session_id")?,
        source_platform: optional_string(object, "source_platform")?,
        source_surface: optional_string(object, "source_surface")?,
        started_at,
        source,
    })
}

fn inspect_content(record: &CanonicalRecord) -> Result<(Option<usize>, Option<bool>), ParseError> {
    let Some(blocks) = &record.message.content else {
        return Ok((None, None));
    };
    let mut count = 0;
    let mut carrier = false;
    for block in blocks {
        match block.get("type").and_then(Value::as_str) {
            Some("text") if block.get("text").and_then(Value::as_str).is_none() => {
                return Err(ParseError::InvalidContent);
            }
            Some("tool_use") => {
                if block
                    .get("name")
                    .and_then(Value::as_str)
                    .is_none_or(|name| name.trim().is_empty())
                {
                    return Err(ParseError::InvalidContent);
                }
                count += 1;
            }
            Some("tool_result") => carrier = true,
            Some(_) => {}
            None => return Err(ParseError::InvalidContent),
        }
    }
    Ok((Some(count), Some(carrier)))
}

fn validate_time(value: Option<&str>) -> Result<(), ParseError> {
    if let Some(value) = value {
        chrono::DateTime::parse_from_rfc3339(value).map_err(|_| ParseError::InvalidTimestamp)?;
    }
    Ok(())
}

fn optional_string(object: &Map<String, Value>, key: &str) -> Result<Option<String>, ParseError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(ParseError::InvalidField),
    }
}
fn required_string(object: &Map<String, Value>, key: &str) -> Result<String, ParseError> {
    optional_string(object, key)?
        .filter(|value| !value.trim().is_empty())
        .ok_or(ParseError::InvalidPrLink)
}
fn optional_u64(object: &Map<String, Value>, key: &str) -> Result<Option<u64>, ParseError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value.as_u64().map(Some).ok_or(ParseError::InvalidField),
    }
}
fn optional_bool(object: &Map<String, Value>, key: &str) -> Result<Option<bool>, ParseError> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(ParseError::InvalidField),
    }
}
