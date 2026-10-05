//! Schema-2 fire rows: whitelisted structural fields only.
//!
//! The row is deserialized into a struct naming only whitelisted keys.
//! Every other key (`excerpt`, `override_reason`, `source_message_id`,
//! `dedup_key`, `raw_matches_before_fire`, anything unknown) is skipped by
//! the parser without being stored. Parser errors are reduced to a reason
//! code: their messages can quote input and are never kept.

use jiff::Timestamp;
use serde::Deserialize;
use serde::de::IgnoredAny;

/// One recorded fire, as structural metadata.
///
/// Strings are the recorded values, unnormalized: `session_id` is the raw
/// host-native identity and `host` the raw recorded host (absent in older
/// rows). Linking to indexed sessions is a later adapter's job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FireRecord {
    pub fire_id: String,
    pub rule_id: String,
    pub rule_version: Option<RuleVersion>,
    pub rulebook_id: Option<String>,
    pub session_id: String,
    pub agent_id: Option<String>,
    /// The plugin's structural checkout hash, not a path.
    pub worktree: Option<String>,
    pub host: Option<String>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub tool: Option<String>,
    pub hook_phase: Option<String>,
    pub mode: FireMode,
    pub fired_at: Timestamp,
}

/// A rule version as recorded: an integer or a short label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleVersion {
    Number(i64),
    Label(String),
}

/// The recorded delivery mode of a row. It is not an outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FireMode {
    /// Advice was delivered; whether it was followed is not recorded.
    Advise,
    /// A gate matched. Blocked and overridden calls are both recorded this
    /// way, so this never proves the call was blocked.
    Gate,
    /// The rule matched but was suppressed; nothing was delivered.
    Suppressed,
    /// Any other recorded value, kept verbatim and never mapped to a known
    /// mode.
    Unrecognized(String),
}

/// Why a line was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reject {
    InvalidJson,
    InvalidShape,
    InvalidTimestamp,
    OversizeValue,
}

/// A whitelisted value before validation. Anything that is neither an
/// integer nor a string is kept only as "other".
#[derive(Deserialize)]
#[serde(untagged)]
enum Value {
    Int(i64),
    Text(String),
    Other(IgnoredAny),
}

/// The whitelisted keys of a schema-2 row. `null` and absence both read as
/// `None`; a repeated whitelisted key fails the whole row.
#[derive(Deserialize)]
struct Wire {
    #[serde(default)]
    fire_id: Option<Value>,
    #[serde(default)]
    rule_id: Option<Value>,
    #[serde(default)]
    rule_version: Option<Value>,
    #[serde(default)]
    rulebook_id: Option<Value>,
    #[serde(default)]
    session_id: Option<Value>,
    #[serde(default)]
    agent_id: Option<Value>,
    #[serde(default)]
    worktree: Option<Value>,
    #[serde(default)]
    host: Option<Value>,
    #[serde(default)]
    repo: Option<Value>,
    #[serde(default)]
    branch: Option<Value>,
    #[serde(default)]
    tool: Option<Value>,
    #[serde(default)]
    hook_phase: Option<Value>,
    #[serde(default)]
    mode: Option<Value>,
    #[serde(default)]
    fired_at: Option<Value>,
}

/// Parse one complete line (without its newline).
pub(super) fn parse(line: &[u8], max_value_bytes: usize) -> Result<FireRecord, Reject> {
    // A derived struct also accepts a JSON array positionally; only an
    // object is a row.
    if line.iter().find(|b| !b.is_ascii_whitespace()) != Some(&b'{') {
        return Err(Reject::InvalidShape);
    }
    let wire: Wire = serde_json::from_slice(line).map_err(|error| {
        if error.is_data() {
            Reject::InvalidShape
        } else {
            Reject::InvalidJson
        }
    })?;
    let limit = Limit(max_value_bytes);
    // Size is checked for every present string first, so an oversize value
    // is reported as such whatever else is wrong with the row.
    for value in [
        &wire.fire_id,
        &wire.rule_id,
        &wire.rule_version,
        &wire.rulebook_id,
        &wire.session_id,
        &wire.agent_id,
        &wire.worktree,
        &wire.host,
        &wire.repo,
        &wire.branch,
        &wire.tool,
        &wire.hook_phase,
        &wire.mode,
        &wire.fired_at,
    ] {
        limit.check(value.as_ref())?;
    }
    let fired_at = instant(&required_text(wire.fired_at)?)?;
    let mode_text = required_text(wire.mode)?;
    let mode = match mode_text.as_str() {
        "advise" => FireMode::Advise,
        "gate" => FireMode::Gate,
        "suppressed" => FireMode::Suppressed,
        _ => FireMode::Unrecognized(mode_text),
    };
    Ok(FireRecord {
        fire_id: required_text(wire.fire_id)?,
        rule_id: required_text(wire.rule_id)?,
        rule_version: match wire.rule_version {
            None => None,
            Some(Value::Int(number)) => Some(RuleVersion::Number(number)),
            Some(Value::Text(label)) => Some(RuleVersion::Label(label)),
            Some(Value::Other(_)) => return Err(Reject::InvalidShape),
        },
        rulebook_id: optional_text(wire.rulebook_id)?,
        session_id: required_text(wire.session_id)?,
        agent_id: optional_text(wire.agent_id)?,
        worktree: optional_text(wire.worktree)?,
        host: optional_text(wire.host)?,
        repo: optional_text(wire.repo)?,
        branch: optional_text(wire.branch)?,
        tool: optional_text(wire.tool)?,
        hook_phase: optional_text(wire.hook_phase)?,
        mode,
        fired_at,
    })
}

struct Limit(usize);

impl Limit {
    fn check(&self, value: Option<&Value>) -> Result<(), Reject> {
        match value {
            Some(Value::Text(text)) if text.len() > self.0 => Err(Reject::OversizeValue),
            _ => Ok(()),
        }
    }
}

/// A present string with at least one non-whitespace character.
fn required_text(value: Option<Value>) -> Result<String, Reject> {
    match value {
        Some(Value::Text(text)) if !text.trim().is_empty() => Ok(text),
        _ => Err(Reject::InvalidShape),
    }
}

fn optional_text(value: Option<Value>) -> Result<Option<String>, Reject> {
    match value {
        None => Ok(None),
        Some(Value::Text(text)) => Ok(Some(text)),
        Some(_) => Err(Reject::InvalidShape),
    }
}

/// An RFC 3339 date-time with a `Z` or `±hh:mm` offset. The shape is
/// checked here because the calendar parser also accepts forms RFC 3339
/// does not (offsets past 23:59, bracketed annotations, omitted parts); the
/// parser then rejects impossible dates and times.
fn instant(text: &str) -> Result<Timestamp, Reject> {
    if !rfc3339_shape(text.as_bytes()) {
        return Err(Reject::InvalidTimestamp);
    }
    text.parse::<Timestamp>()
        .map_err(|_| Reject::InvalidTimestamp)
}

fn rfc3339_shape(bytes: &[u8]) -> bool {
    let digits = |range: std::ops::Range<usize>| {
        bytes
            .get(range)
            .is_some_and(|part| part.iter().all(u8::is_ascii_digit))
    };
    let at = |index: usize, allowed: &[u8]| bytes.get(index).is_some_and(|b| allowed.contains(b));
    let date_time = digits(0..4)
        && at(4, b"-")
        && digits(5..7)
        && at(7, b"-")
        && digits(8..10)
        && at(10, b"Tt")
        && digits(11..13)
        && at(13, b":")
        && digits(14..16)
        && at(16, b":")
        && digits(17..19);
    if !date_time {
        return false;
    }
    let mut rest = &bytes[19..];
    if let Some(fraction) = rest.strip_prefix(b".") {
        let len = fraction.iter().take_while(|b| b.is_ascii_digit()).count();
        if !(1..=9).contains(&len) {
            return false;
        }
        rest = &fraction[len..];
    }
    match rest {
        [b'Z' | b'z'] => true,
        [b'+' | b'-', h1, h2, b':', m1, m2] => {
            let hours = [*h1, *h2];
            let minutes = [*m1, *m2];
            hours.iter().chain(&minutes).all(u8::is_ascii_digit)
                && hours <= *b"23"
                && minutes <= *b"59"
        }
        _ => false,
    }
}
