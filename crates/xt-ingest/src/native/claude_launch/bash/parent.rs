//! Reading one saved primary Claude history, from its first byte, for what
//! the Bash launch child facts need: what its opening says (the first
//! eligible input, for the possible children of a launch) and the `Bash`
//! calls in it that launch Claude print sessions (for an indexed caller).
//! Reused from the reviewed parked reader; only its relation labels are gone.
//!
//! The history is read forward in budgeted chunks as the generation it was
//! opened at, complete lines only, and is done only when every byte of that
//! generation is a complete line: a history whose last line is still being
//! written decides nothing until it changes. A changed generation is always
//! read again from the start — whatever changed, appended to or rewritten —
//! and nothing read before is carried over.
//!
//! Every record has a native [`Context`]: the session it names and its chain
//! (the main conversation, or one agent's sidechain). A record that names no
//! session, names a blank one, does not say which chain it is on, or is on a
//! sidechain without a nonblank agent has an unknown context. The history's
//! [`Identity`] is the one session all its records name, if they name one
//! and only one; a blank name makes it not one.
//!
//! A call counts when any assistant record holds a `Bash` tool call whose
//! input is only a command (with an optional description, timeout or sandbox
//! switch, never in the background) that [`script::parse`] reads. Every such
//! occurrence is kept as evidence of a possible creator, whatever its
//! context and however often its identifier recurs, each with its own
//! record. A result closes a call — ends the window in which it could have
//! created a child — only when it is a user record in the call's very own,
//! known context, after the call, dated at or after it: anything else leaves
//! the call open. A call names a child only when its identifier occurs as
//! exactly one call and one closing result, both on the main chain of the
//! history's own verified identity, the result the only tool result in its
//! record, not an error, its text exactly the recorded standard output, with
//! no standard error and not interrupted. Every call and result identifier
//! anywhere in the history is counted, whatever its record or order.
//!
//! What is kept per call is its records' identifiers and times, each
//! launch's directory and prompt, and, once its result is read, each launch's
//! printed answer as an in-memory digest and the identifier a JSON result
//! named. A launch printing streaming JSON or verbose output keeps its call
//! and place, but its output is never read as an answer, so it decides no
//! child.
//! Every retained buffer and identifier is reserved from the shared
//! [`Allowance`] first.

use super::super::options::Format;
use super::super::rows;
use super::super::source::{self, Allowance, Budget, Lines, Reserved, Unread};
use super::script::{self, Script};
use serde_json::Value;
use sha2::Digest;
use std::{
    collections::HashMap,
    fs::File,
    path::{Path, PathBuf},
};
use xt_store::claude_launch::SegmentGeneration;

/// Supported calls one history may hold, and their launches.
const MAX_CALLS: usize = 256;
const MAX_INVOCATIONS: usize = 1_000;
/// What one counted identifier is reserved at beyond its text: its slot and
/// the table's spare room.
const ID_SLOT: usize = 2 * (std::mem::size_of::<(String, (u32, u32))>() + 1);

/// What a history's opening says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum First {
    /// The complete history holds no eligible input.
    NoInput,
    Input(FirstInput),
    /// It cannot be read now, or is beyond the bounds.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct FirstInput {
    pub uuid: Option<String>,
    pub ts_ms: Option<i64>,
    pub cwd: Option<String>,
    /// The digest of its one text, when it has exactly one.
    pub digest: Option<[u8; 32]>,
}

impl FirstInput {
    pub fn bytes(&self) -> usize {
        self.uuid.as_ref().map_or(0, String::capacity)
            + self.cwd.as_ref().map_or(0, String::capacity)
    }
}

/// What one launch printed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Output {
    /// The output cannot name a child.
    Rejected,
    /// The complete answer's digest, and the child a JSON result named.
    Answer {
        digest: [u8; 32],
        returned: Option<String>,
    },
}

/// A call's result, as far as the history holds one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum CallResult {
    Open,
    /// A result was recorded but cannot be read as this call's output.
    Rejected {
        uuid: String,
        ms: i64,
    },
    Printed {
        uuid: String,
        ms: i64,
        outputs: Vec<Output>,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Call {
    pub id: String,
    pub launch_uuid: String,
    pub launch_ms: i64,
    pub launches: Vec<script::Launch>,
    /// What the script prints, until its result is read.
    items: Option<Vec<script::Item>>,
    pub result: CallResult,
    /// The identifier does not occur as exactly one call and one result.
    pub duplicate: bool,
    /// The native context of its record.
    context: Context,
}

/// Whether a native session or agent label names anything: the rule the
/// native reader and writer already apply, a label that is empty once its
/// surrounding white space is set aside names nothing. A label that does
/// name something is kept exactly as written.
fn label(value: &str) -> bool {
    !value.trim().is_empty()
}

/// Which conversation a record belongs to, as the record itself says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Context {
    /// The main chain of this session.
    Main(String),
    /// This agent's sidechain of this session.
    Side(String, String),
    Unknown,
}

impl Context {
    fn of(value: &Value) -> Self {
        let Some(session) = value
            .get("sessionId")
            .and_then(Value::as_str)
            .filter(|id| label(id))
        else {
            return Self::Unknown;
        };
        match value.get("isSidechain").and_then(Value::as_bool) {
            Some(false) => Self::Main(session.to_owned()),
            Some(true) => match value
                .get("agentId")
                .and_then(Value::as_str)
                .filter(|id| label(id))
            {
                Some(agent) => Self::Side(session.to_owned(), agent.to_owned()),
                None => Self::Unknown,
            },
            None => Self::Unknown,
        }
    }

    fn bytes(&self) -> usize {
        match self {
            Self::Main(session) => session.capacity(),
            Self::Side(session, agent) => session.capacity() + agent.capacity(),
            Self::Unknown => 0,
        }
    }
}

/// The session a history's records name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Identity {
    /// No record named a session.
    Unnamed,
    /// Every record that names a session names this one.
    One(String),
    /// Records name more than one session, or name one unreadably.
    Mixed,
}

impl Identity {
    /// Fold in what one record names; `true` when a new name was kept.
    fn see(&mut self, session: Option<&Value>) -> bool {
        match (session, &*self) {
            (None, _) | (_, Self::Mixed) => false,
            // A blank label names no session: the identity is not one.
            (Some(Value::String(named)), _) if !label(named) => {
                *self = Self::Mixed;
                false
            }
            (Some(Value::String(named)), Self::One(known)) if named == known => false,
            (Some(Value::String(named)), Self::Unnamed) => {
                *self = Self::One(named.clone());
                true
            }
            _ => {
                *self = Self::Mixed;
                false
            }
        }
    }

    /// The one session, when the records name exactly one.
    pub fn verified(&self) -> Option<&str> {
        match self {
            Self::One(session) => Some(session),
            _ => None,
        }
    }
}

/// What every record is read for, without the rest of it.
#[derive(serde::Deserialize)]
struct Head {
    #[serde(rename = "sessionId")]
    session: Option<Value>,
}

impl Call {
    /// Bytes its strings hold, at about their capacity.
    fn bytes(&self) -> usize {
        let launches: usize = self
            .launches
            .iter()
            .map(|launch| {
                launch.cwd.capacity()
                    + launch.prompt.capacity()
                    + launch.session_id.as_ref().map_or(0, String::capacity)
                    + std::mem::size_of::<script::Launch>()
            })
            .sum();
        let items: usize = self
            .items
            .iter()
            .flatten()
            .map(|item| match item {
                script::Item::Echo(text) => text.capacity() + 32,
                script::Item::Launch(_) => 32,
            })
            .sum();
        std::mem::size_of::<Call>()
            + self.context.bytes()
            + self.id.capacity()
            + self.launch_uuid.capacity()
            + launches
            + items
            + 256
    }
}

/// One flattened invocation, as decisions need it.
#[derive(Clone, Debug)]
pub(super) struct Invocation {
    pub call_id: String,
    pub index: u32,
    pub launch_uuid: String,
    pub launch_ms: i64,
    /// The result record and its time, when the call has a readable one.
    pub result: Option<(String, i64)>,
    /// The record holding the call's result, readable or not.
    pub result_record: Option<String>,
    /// The end of the window a child of this invocation may be born in:
    /// `None` while the call has no result, or cannot name one.
    pub end_ms: Option<i64>,
    pub cwd: String,
    pub prompt: String,
    pub prompt_digest: [u8; 32],
    /// The child the command or its JSON result named.
    pub named: Option<String>,
    /// The printed answer's digest, when the invocation can name a child.
    pub answer: Option<[u8; 32]>,
}

impl Invocation {
    pub fn bytes(&self) -> usize {
        std::mem::size_of::<Invocation>()
            + self.call_id.capacity()
            + self.launch_uuid.capacity()
            + self.result.as_ref().map_or(0, |(uuid, _)| uuid.capacity())
            + self.result_record.as_ref().map_or(0, String::capacity)
            + self.cwd.capacity()
            + self.prompt.capacity()
            + self.named.as_ref().map_or(0, String::capacity)
    }
}

/// Why a history is not read now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ScanError {
    /// Busy or changed: read again later.
    Changed,
    /// Its last line is still being written at this generation.
    Unfinished,
    /// Beyond this version's bounds, or not a file it reads.
    Refused,
    /// The shared allowance cannot hold it now.
    Memory,
}

/// What a finished read found.
#[derive(Debug)]
pub(super) struct Facts {
    pub generation: SegmentGeneration,
    pub first: First,
    /// Every supported invocation; `None` when only the opening was read.
    pub invocations: Option<Vec<Invocation>>,
}

#[derive(Debug)]
pub(super) struct ParentScan {
    pub path: PathBuf,
    native: String,
    programs: Vec<PathBuf>,
    file: File,
    pub generation: SegmentGeneration,
    /// Only the opening is wanted: the history's calls are already known.
    opening_only: bool,
    lines: Lines,
    allowance: Allowance,
    /// Every call and result identifier seen: how often as each.
    ids: HashMap<String, (u32, u32)>,
    calls: Vec<Call>,
    /// Every occurrence of each supported call's identifier.
    by_id: HashMap<String, Vec<usize>>,
    /// The identifiers and calls kept.
    held: Reserved,
    first: Option<First>,
    identity: Identity,
}

fn token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && value.bytes().all(|byte| byte.is_ascii_graphic())
}

impl ParentScan {
    /// Open `path` as the history of session `native` (its file name).
    pub fn new(
        path: &Path,
        native: &str,
        programs: &[PathBuf],
        opening_only: bool,
        max_line: usize,
        allowance: &Allowance,
    ) -> Result<Self, ScanError> {
        let (file, generation) = source::open(path).map_err(refusal)?;
        let held = allowance
            .reserve(path.as_os_str().len() + native.len() + std::mem::size_of::<Self>())
            .map_err(refusal)?;
        Ok(Self {
            path: path.to_path_buf(),
            native: native.to_owned(),
            programs: programs.to_vec(),
            file,
            generation,
            opening_only,
            lines: Lines::new(max_line, allowance).map_err(refusal)?,
            allowance: allowance.clone(),
            ids: HashMap::new(),
            calls: Vec::new(),
            by_id: HashMap::new(),
            held,
            first: None,
            identity: Identity::Unnamed,
        })
    }

    /// Read on within `budget`: `Ok(None)` when paused, else what the whole
    /// generation says.
    pub fn advance(&mut self, budget: &mut Budget<'_>) -> Result<Option<Facts>, ScanError> {
        let len = u64::try_from(self.generation.length).map_err(|_| ScanError::Refused)?;
        while self.lines.next < len && !(self.opening_only && self.first.is_some()) {
            let chunk = match source::read_chunk(
                &self.file,
                self.lines.next,
                len - self.lines.next,
                budget,
            ) {
                Ok(Some(chunk)) => chunk,
                Ok(None) => return Ok(None),
                Err(error) => return Err(refusal(error)),
            };
            let Self {
                lines,
                programs,
                ids,
                calls,
                by_id,
                held,
                first,
                identity,
                allowance,
                opening_only,
                ..
            } = self;
            let mut failed: Option<ScanError> = None;
            lines
                .feed(&chunk, &mut |_, bytes| {
                    if failed.is_some() || (*opening_only && first.is_some()) {
                        return Ok(());
                    }
                    let mut state = LineState {
                        programs,
                        ids,
                        calls,
                        by_id,
                        held,
                        allowance,
                    };
                    if let Err(error) = state.line(bytes, first, identity, *opening_only) {
                        failed = Some(error);
                    }
                    Ok(())
                })
                .map_err(refusal)?;
            if let Some(error) = failed {
                return Err(error);
            }
        }
        // Every byte of this generation must be a complete line, unless only
        // the opening was wanted and it was found.
        let opened = self.opening_only && self.first.is_some();
        if !opened && !self.lines.finished() {
            return Err(ScanError::Unfinished);
        }
        // Still exactly the generation that was read.
        if source::generation_of(&self.file).ok() != Some(self.generation)
            || source::stat(&self.path).ok() != Some(self.generation)
        {
            return Err(ScanError::Changed);
        }
        let first = self.first.clone().unwrap_or(First::NoInput);
        if self.opening_only {
            return Ok(Some(Facts {
                generation: self.generation,
                first,
                invocations: None,
            }));
        }
        // A call names a child only through one call and one result.
        for call in &mut self.calls {
            let answered = u32::from(call.result != CallResult::Open);
            if self.ids.get(&call.id) != Some(&(1, answered)) {
                call.duplicate = true;
            }
        }
        Ok(Some(Facts {
            generation: self.generation,
            first,
            invocations: Some(self.invocations()),
        }))
    }

    /// Every supported invocation, in call then launch order.
    fn invocations(&self) -> Vec<Invocation> {
        let mut out = Vec::new();
        for call in &self.calls {
            // Only a unique, closed call on the main chain of the history's
            // own verified session can name a child; every occurrence is
            // evidence.
            let own = matches!(&call.context, Context::Main(session)
                if *session == self.native && self.identity.verified() == Some(session.as_str()));
            let names = !call.duplicate && own && call.result != CallResult::Open;
            let result = match &call.result {
                CallResult::Printed { uuid, ms, .. } if names => Some((uuid.clone(), *ms)),
                _ => None,
            };
            let result_record = match &call.result {
                CallResult::Printed { uuid, .. } | CallResult::Rejected { uuid, .. } => {
                    Some(uuid.clone())
                }
                CallResult::Open => None,
            };
            let end_ms = match &call.result {
                _ if call.duplicate => None,
                CallResult::Printed { ms, .. } | CallResult::Rejected { ms, .. } => Some(*ms),
                CallResult::Open => None,
            };
            for (index, launch) in call.launches.iter().enumerate() {
                let output = match &call.result {
                    CallResult::Printed { outputs, .. } if names => outputs.get(index),
                    _ => None,
                };
                let (answer, named) = match output {
                    Some(Output::Answer { digest, returned }) => (
                        Some(*digest),
                        launch.session_id.clone().or_else(|| returned.clone()),
                    ),
                    _ => (None, launch.session_id.clone()),
                };
                out.push(Invocation {
                    call_id: call.id.clone(),
                    index: index as u32,
                    launch_uuid: call.launch_uuid.clone(),
                    launch_ms: call.launch_ms,
                    result: result.clone(),
                    result_record: result_record.clone(),
                    end_ms,
                    cwd: launch.cwd.clone(),
                    prompt: launch.prompt.clone(),
                    prompt_digest: sha2::Sha256::digest(launch.prompt.as_bytes()).into(),
                    named,
                    answer,
                });
            }
        }
        out
    }
}

fn refusal(error: Unread) -> ScanError {
    match error {
        Unread::Missing | Unread::Unreadable | Unread::Changed => ScanError::Changed,
        Unread::Unfinished => ScanError::Unfinished,
        Unread::Memory => ScanError::Memory,
        Unread::Alias | Unread::TooLarge => ScanError::Refused,
    }
}

/// What one line folds into.
struct LineState<'a> {
    programs: &'a [PathBuf],
    ids: &'a mut HashMap<String, (u32, u32)>,
    calls: &'a mut Vec<Call>,
    by_id: &'a mut HashMap<String, Vec<usize>>,
    held: &'a mut Reserved,
    allowance: &'a Allowance,
}

impl LineState<'_> {
    /// Count one more call or result of `id`, reserving a new one first.
    fn count(&mut self, id: &str, result: bool) -> Result<(), ScanError> {
        if !self.ids.contains_key(id) {
            self.held.grow(id.len() + ID_SLOT).map_err(refusal)?;
            self.ids.insert(id.to_owned(), (0, 0));
        }
        let counts = self.ids.get_mut(id).expect("inserted");
        if result {
            counts.1 = counts.1.saturating_add(1);
        } else {
            counts.0 = counts.0.saturating_add(1);
        }
        Ok(())
    }

    fn line(
        &mut self,
        bytes: &[u8],
        first: &mut Option<First>,
        identity: &mut Identity,
        opening_only: bool,
    ) -> Result<(), ScanError> {
        let Ok(text) = std::str::from_utf8(bytes) else {
            return Ok(());
        };
        // Every record's session, whatever else it holds. A line that is not
        // a JSON object is not a record.
        if let Ok(head) = serde_json::from_str::<Head>(text)
            && identity.see(head.session.as_ref())
        {
            self.held
                .grow(identity.verified().map_or(0, str::len))
                .map_err(refusal)?;
        }
        let structural = text.contains("tool_use");
        if first.is_none() || (structural && !opening_only) {
            let _decoding = self
                .allowance
                .reserve(source::decoded_bound(bytes))
                .map_err(refusal)?;
            let Ok(value) = serde_json::from_str::<Value>(text) else {
                return Ok(());
            };
            if first.is_none() {
                let opening = opening(&value);
                if let Some(First::Input(input)) = &opening {
                    self.held.grow(input.bytes()).map_err(refusal)?;
                }
                *first = opening;
            }
            if structural && !opening_only {
                self.structure(&value)?;
            }
        }
        Ok(())
    }

    /// Count every call and result in a record, and read the supported ones.
    fn structure(&mut self, value: &Value) -> Result<(), ScanError> {
        let context = Context::of(value);
        let kind = value.get("type").and_then(Value::as_str);
        let Some(blocks) = value["message"]["content"].as_array() else {
            return Ok(());
        };
        let results = blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("tool_result"))
            .count();
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    let Some(id) = block.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    self.count(id, false)?;
                    if kind != Some("assistant")
                        || block.get("name").and_then(Value::as_str) != Some("Bash")
                    {
                        continue;
                    }
                    let Some(mut call) = launch(value, id, block, self.programs) else {
                        continue;
                    };
                    call.context = context.clone();
                    let launches: usize = self.calls.iter().map(|call| call.launches.len()).sum();
                    if self.calls.len() >= MAX_CALLS
                        || launches + call.launches.len() > MAX_INVOCATIONS
                    {
                        return Err(ScanError::Refused);
                    }
                    self.held
                        .grow(call.bytes() + id.len() + 64)
                        .map_err(refusal)?;
                    let at = self.calls.len();
                    let occurrences = self.by_id.entry(id.to_owned()).or_default();
                    if occurrences.capacity() == occurrences.len() {
                        self.held
                            .grow(std::mem::size_of::<usize>() * occurrences.len().max(4))
                            .map_err(refusal)?;
                    }
                    occurrences.push(at);
                    self.calls.push(call);
                }
                Some("tool_result") => {
                    let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else {
                        continue;
                    };
                    self.count(id, true)?;
                    // A result answers only an identifier called once; any
                    // other is withheld by the counts.
                    let Some([at]) = self.by_id.get(id).map(Vec::as_slice) else {
                        continue;
                    };
                    let call = &mut self.calls[*at];
                    // Only a later result in the call's own known context,
                    // dated at or after it, closes it.
                    let closes = call.result == CallResult::Open
                        && kind == Some("user")
                        && call.context != Context::Unknown
                        && context == call.context
                        && rows::timestamp(value).is_some_and(|ms| ms >= call.launch_ms);
                    if !closes {
                        continue;
                    }
                    call.result = result(value, block, results, call);
                    call.items = None;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

/// What one record says about a history's opening: `None` while it is not
/// an eligible input (a user record that is neither meta, sidechain nor a
/// tool result).
pub(super) fn opening(value: &Value) -> Option<First> {
    let eligible = value.get("type").and_then(Value::as_str) == Some("user")
        && value.get("isMeta").and_then(Value::as_bool) != Some(true)
        && value.get("isSidechain").and_then(Value::as_bool) != Some(true)
        && value["message"].get("role").and_then(Value::as_str) == Some("user");
    if !eligible {
        return None;
    }
    let text = match &value["message"]["content"] {
        Value::String(text) => Some(text.as_str()),
        Value::Array(parts)
            if parts
                .iter()
                .any(|part| part.get("type").and_then(Value::as_str) == Some("tool_result")) =>
        {
            return None;
        }
        Value::Array(parts) => match parts.as_slice() {
            [part] if part.get("type").and_then(Value::as_str) == Some("text") => {
                part.get("text").and_then(Value::as_str)
            }
            _ => None,
        },
        _ => None,
    };
    Some(First::Input(FirstInput {
        uuid: value.get("uuid").and_then(Value::as_str).map(str::to_owned),
        ts_ms: rows::timestamp(value),
        cwd: value.get("cwd").and_then(Value::as_str).map(str::to_owned),
        digest: text.map(|text| sha2::Sha256::digest(text.as_bytes()).into()),
    }))
}

/// A supported call, from its assistant record and block.
fn launch(value: &Value, id: &str, block: &Value, programs: &[PathBuf]) -> Option<Call> {
    let uuid = value
        .get("uuid")
        .and_then(Value::as_str)
        .filter(|uuid| token(uuid))?;
    let launch_ms = rows::timestamp(value)?;
    if !token(id) {
        return None;
    }
    let input = block.get("input")?.as_object()?;
    for (key, field) in input {
        let allowed = match key.as_str() {
            "command" | "description" => field.is_string(),
            "timeout" => field.is_number(),
            "dangerouslyDisableSandbox" => field.is_boolean(),
            "run_in_background" => field.as_bool() == Some(false),
            _ => false,
        };
        if !allowed {
            return None;
        }
    }
    let command = input.get("command")?.as_str()?;
    let Script { items, launches } = script::parse(command, programs)?;
    Some(Call {
        id: id.to_owned(),
        launch_uuid: uuid.to_owned(),
        launch_ms,
        launches,
        items: Some(items),
        result: CallResult::Open,
        duplicate: false,
        context: Context::Unknown,
    })
}

/// The call's result, read from its record and block.
fn result(value: &Value, block: &Value, results: usize, call: &Call) -> CallResult {
    let (Some(uuid), Some(ms)) = (
        value
            .get("uuid")
            .and_then(Value::as_str)
            .filter(|uuid| token(uuid)),
        rows::timestamp(value),
    ) else {
        return CallResult::Open;
    };
    let rejected = CallResult::Rejected {
        uuid: uuid.to_owned(),
        ms,
    };
    let recorded = &value["toolUseResult"];
    let printed = block.get("content").and_then(Value::as_str);
    let clean = results == 1
        && ms >= call.launch_ms
        && block.get("is_error").and_then(Value::as_bool) != Some(true)
        && printed.is_some()
        && recorded.get("stdout").and_then(Value::as_str) == printed
        && recorded.get("stderr").and_then(Value::as_str) == Some("")
        && recorded.get("interrupted").and_then(Value::as_bool) == Some(false)
        && recorded.get("isImage").and_then(Value::as_bool) != Some(true);
    let (Some(printed), Some(items), true) = (printed, call.items.as_ref(), clean) else {
        return rejected;
    };
    let script = Script {
        items: items.clone(),
        launches: call.launches.clone(),
    };
    let Some(chunks) = script::partition(&script, printed) else {
        return rejected;
    };
    let outputs = chunks
        .iter()
        .zip(&call.launches)
        .map(|(chunk, launch)| match (launch.format, launch.verbose) {
            (Format::Text, false) => Output::Answer {
                digest: sha2::Sha256::digest(chunk.as_bytes()).into(),
                returned: None,
            },
            (Format::Json, false) => json_answer(chunk, launch.session_id.as_deref()),
            // Streaming events and verbose output are neither one plain
            // answer nor one JSON result: recognized, never matched.
            (Format::StreamJson, _) | (_, true) => Output::Rejected,
        })
        .collect();
    CallResult::Printed {
        uuid: uuid.to_owned(),
        ms,
        outputs,
    }
}

/// Claude's own JSON result on one line: a successful result naming a full
/// session identifier, holding its answer as text and the turn count and
/// duration Claude always writes. Anything else, or an identifier other than
/// the one the command supplied, names no child.
fn json_answer(chunk: &str, supplied: Option<&str>) -> Output {
    if chunk.contains('\n') {
        return Output::Rejected;
    }
    let Ok(Value::Object(result)) = serde_json::from_str::<Value>(chunk) else {
        return Output::Rejected;
    };
    let session = result.get("session_id").and_then(Value::as_str);
    let answer = result.get("result").and_then(Value::as_str);
    let native = result.get("type").and_then(Value::as_str) == Some("result")
        && result.get("subtype").and_then(Value::as_str) == Some("success")
        && result.get("is_error").and_then(Value::as_bool) == Some(false)
        && result.get("num_turns").and_then(Value::as_u64).is_some()
        && result.get("duration_ms").is_some_and(Value::is_number);
    match (native, session, answer) {
        (true, Some(session), Some(answer))
            if super::super::command::uuid(session)
                && supplied.is_none_or(|supplied| supplied == session) =>
        {
            Output::Answer {
                digest: sha2::Sha256::digest(answer.as_bytes()).into(),
                returned: Some(session.to_owned()),
            }
        }
        _ => Output::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Only a plain answer or one JSON result is read from what a launch
    /// printed. Streaming events, and anything a verbose launch printed, are
    /// never matched — even a line that reads exactly as a JSON result.
    /// Streaming output without `--verbose` is no launch at all: the CLI
    /// refuses it before starting any session.
    #[test]
    fn only_a_plain_answer_or_one_json_result_is_read_as_output() {
        const CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";
        let result_line = json!({
            "type": "result", "subtype": "success", "is_error": false, "num_turns": 1,
            "duration_ms": 5, "session_id": CHILD, "result": "Done."
        })
        .to_string();
        let digest = |text: &str| -> [u8; 32] { sha2::Sha256::digest(text.as_bytes()).into() };
        for (options, printed, expected) in [
            (
                "",
                "Done.".to_owned(),
                Output::Answer {
                    digest: digest("Done."),
                    returned: None,
                },
            ),
            (
                "--output-format json",
                result_line.clone(),
                Output::Answer {
                    digest: digest("Done."),
                    returned: Some(CHILD.into()),
                },
            ),
            (
                "--output-format stream-json --verbose",
                result_line.clone(),
                Output::Rejected,
            ),
            (
                "--output-format json --verbose",
                result_line.clone(),
                Output::Rejected,
            ),
            ("--verbose", "Done.".to_owned(), Output::Rejected),
        ] {
            let command = format!("(cd /a && claude -p q {options})");
            let call_record = json!({"uuid": "u-call", "timestamp": "2026-10-04T00:00:00.000Z"});
            let call_block = json!({"input": {"command": command}});
            let call = launch(&call_record, "toolu_1", &call_block, &[]).expect(&command);
            let result_record = json!({
                "uuid": "u-result", "timestamp": "2026-10-04T00:00:01.000Z",
                "toolUseResult": {"stdout": printed, "stderr": "", "interrupted": false}
            });
            let result_block = json!({"content": printed});
            assert_eq!(
                result(&result_record, &result_block, 1, &call),
                CallResult::Printed {
                    uuid: "u-result".into(),
                    ms: rows::timestamp(&result_record).unwrap(),
                    outputs: vec![expected],
                },
                "{command}"
            );
        }
        let refused =
            json!({"input": {"command": "(cd /a && claude -p q --output-format stream-json)"}});
        let record = json!({"uuid": "u-call", "timestamp": "2026-10-04T00:00:00.000Z"});
        assert!(launch(&record, "toolu_1", &refused, &[]).is_none());
    }

    #[test]
    fn a_blank_session_or_agent_label_is_no_context_and_no_identity() {
        let main = |session: &str| json!({"sessionId": session, "isSidechain": false});
        let side = |agent: &str| json!({"sessionId": "s", "isSidechain": true, "agentId": agent});
        for blank in ["", " ", "\t\n "] {
            assert_eq!(Context::of(&main(blank)), Context::Unknown, "{blank:?}");
            assert_eq!(Context::of(&side(blank)), Context::Unknown, "{blank:?}");
            let mut identity = Identity::Unnamed;
            assert!(!identity.see(Some(&json!(blank))));
            assert_eq!(identity, Identity::Mixed);
            assert_eq!(identity.verified(), None);
            let mut identity = Identity::One("s".into());
            identity.see(Some(&json!(blank)));
            assert_eq!(identity, Identity::Mixed);
        }
        // A label that names something is kept exactly, never trimmed into
        // another one.
        assert_eq!(Context::of(&main(" s ")), Context::Main(" s ".into()));
        assert_eq!(
            Context::of(&side(" a")),
            Context::Side("s".into(), " a".into())
        );
        let mut identity = Identity::Unnamed;
        identity.see(Some(&json!(" s ")));
        assert_eq!(identity.verified(), Some(" s "));
        identity.see(Some(&json!("s")));
        assert_eq!(identity, Identity::Mixed);
    }
}
