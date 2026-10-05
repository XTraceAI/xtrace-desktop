//! Only the identity, revision and tiny runtime status survive deserialization;
//! `projection` removes everything else before serde sees the frame.
use crate::dto::LiveSessionState;
use serde::{
    Deserialize, Deserializer,
    de::{SeqAccess, Visitor},
};
use serde_json::{Value, json};
use std::fmt;

/// A length prefix above this is treated as a broken stream, not a frame.
/// Bodies are streamed through a bounded projection and never held, so this
/// bounds work per frame, not memory: 15x the largest snapshot observed (17MB).
pub(super) const MAX_FRAME: usize = 256 * 1024 * 1024;
const MAX_PATCHES: usize = 256;
const MAX_SAFE_REVISION: u64 = (1 << 53) - 1;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Message {
    #[serde(rename = "type")]
    pub kind: String,
    pub request_id: Option<String>,
    pub source_client_id: Option<String>,
    pub handled_by_client_id: Option<String>,
    #[serde(default, deserialize_with = "optional_version")]
    pub version: Option<u64>,
    pub method: Option<String>,
    pub result_type: Option<String>,
    pub result: Option<InitializeResult>,
    pub params: Option<Params>,
    pub client_id: Option<String>,
    pub status: Option<Value>,
}

// The producer omits response versions. An explicit null or malformed value
// is still invalid; Option's normal decoder would treat null as omission.
fn optional_version<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct InitializeResult {
    pub client_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Params {
    pub conversation_id: Option<String>,
    pub host_id: Option<String>,
    pub change: Option<Change>,
    pub client_id: Option<String>,
    pub status: Option<Value>,
    pub connected: Option<bool>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub(super) enum Change {
    #[serde(rename_all = "camelCase")]
    Snapshot {
        revision: u64,
        conversation_state: Projection,
    },
    #[serde(rename_all = "camelCase")]
    Patches {
        base_revision: u64,
        revision: u64,
        #[serde(deserialize_with = "bounded_patches")]
        patches: Vec<Patch>,
    },
    #[serde(other)]
    Unsupported,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Projection {
    pub id: String,
    pub host_id: String,
    pub thread_runtime_status: Value,
}

#[derive(Deserialize)]
pub(super) struct Patch {
    op: String,
    path: Value,
    value: Option<Value>,
}

fn bounded_patches<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<Patch>, D::Error> {
    struct Patches;
    impl<'de> Visitor<'de> for Patches {
        type Value = Vec<Patch>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("at most 256 patches")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(patch) = seq.next_element()? {
                if out.len() == MAX_PATCHES {
                    return Err(serde::de::Error::custom("patch bound"));
                }
                out.push(patch);
            }
            Ok(out)
        }
    }
    d.deserialize_seq(Patches)
}

/// Whole-body parse through the same projection the socket streams through.
#[cfg(test)]
pub(super) fn parse(bytes: &[u8]) -> Result<Message, ()> {
    if bytes.is_empty() || bytes.len() > MAX_FRAME {
        return Err(());
    }
    let mut scanner = super::projection::Scanner::default();
    scanner.feed(bytes).map_err(|_| ())?;
    from_projection(&scanner.finish().map_err(|_| ())?)
}

/// The projection holds only status fields; serde never sees transcript
/// content, and still enforces every type, version and patch bound.
pub(super) fn from_projection(bytes: &[u8]) -> Result<Message, ()> {
    serde_json::from_slice(bytes).map_err(|_| ())
}

pub(super) fn client_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flag {
    Approval,
    Input,
}

#[derive(Clone, Debug)]
enum Runtime {
    Active(Vec<Flag>),
    Idle,
}

impl Runtime {
    fn parse(value: &Value) -> Option<Self> {
        let kind = value.get("type")?.as_str()?;
        match kind {
            "idle"
                if value
                    .get("activeFlags")
                    .is_none_or(|v| v.as_array().is_some_and(Vec::is_empty)) =>
            {
                Some(Self::Idle)
            }
            "active" => {
                let flags = value.get("activeFlags")?.as_array()?;
                if flags.len() > 2 {
                    return None;
                }
                let mut out = Vec::new();
                for flag in flags {
                    let flag = match flag.as_str()? {
                        "waitingOnApproval" => Flag::Approval,
                        "waitingOnUserInput" => Flag::Input,
                        _ => return None,
                    };
                    if out.contains(&flag) {
                        return None;
                    }
                    out.push(flag);
                }
                Some(Self::Active(out))
            }
            _ => None,
        }
    }

    fn status(&self) -> LiveSessionState {
        match self {
            Self::Idle => LiveSessionState::Idle,
            Self::Active(flags) if flags.contains(&Flag::Approval) => {
                LiveSessionState::WaitingApproval
            }
            Self::Active(flags) if flags.contains(&Flag::Input) => LiveSessionState::WaitingInput,
            Self::Active(_) => LiveSessionState::Running,
        }
    }

    fn value(&self) -> Value {
        match self {
            Self::Idle => json!({"type":"idle"}),
            Self::Active(flags) => {
                json!({"type":"active","activeFlags":flags.iter().map(|f| match f {
                Flag::Approval => "waitingOnApproval", Flag::Input => "waitingOnUserInput",
            }).collect::<Vec<_>>()})
            }
        }
    }
}

#[derive(Default)]
pub(super) struct ThreadStatus {
    revision: Option<u64>,
    runtime: Option<Runtime>,
}

impl ThreadStatus {
    pub fn clear(&mut self) {
        self.runtime = None;
    }
    /// A new connection, owner or subscription starts a new revision history.
    pub fn reset(&mut self) {
        self.revision = None;
        self.clear();
    }
    pub fn status(&self) -> LiveSessionState {
        self.runtime
            .as_ref()
            .map(Runtime::status)
            .unwrap_or_default()
    }
    pub fn needs_snapshot(&self) -> bool {
        self.runtime.is_none()
    }

    /// Returns false on any unsupported/gapped/identity-changing update. The
    /// caller resubscribes on a bounded timer; patches cannot heal a gap.
    pub fn apply(&mut self, native: &str, change: Change) -> bool {
        let result = match change {
            Change::Snapshot {
                revision,
                conversation_state,
            } => {
                if revision > MAX_SAFE_REVISION
                    || self.revision.is_some_and(|old| revision < old)
                    || conversation_state.id != native
                    || conversation_state.host_id != "local"
                {
                    None
                } else {
                    Runtime::parse(&conversation_state.thread_runtime_status)
                        .map(|runtime| (revision, runtime))
                }
            }
            Change::Patches {
                base_revision,
                revision,
                patches,
            } => {
                if self.revision != Some(base_revision)
                    || revision <= base_revision
                    || revision > MAX_SAFE_REVISION
                {
                    None
                } else {
                    self.runtime.as_ref().and_then(|runtime| {
                        let mut value = runtime.value();
                        for patch in patches {
                            apply_patch(&mut value, native, patch)?;
                        }
                        Runtime::parse(&value).map(|runtime| (revision, runtime))
                    })
                }
            }
            Change::Unsupported => None,
        };
        if let Some((revision, runtime)) = result {
            self.revision = Some(revision);
            self.runtime = Some(runtime);
            true
        } else {
            self.clear();
            false
        }
    }
}

pub(super) fn path(value: Value) -> Option<Vec<String>> {
    match value {
        Value::Array(tokens) if tokens.len() <= 16 => tokens
            .into_iter()
            .map(|v| match v {
                Value::String(s) if s.len() <= 128 => Some(s),
                Value::Number(n) => n.as_u64().map(|n| n.to_string()),
                _ => None,
            })
            .collect(),
        Value::String(s) if s.is_empty() => Some(Vec::new()),
        Value::String(s) if s.starts_with('/') && s.len() <= 1024 => s[1..]
            .split('/')
            .map(|part| {
                let mut out = String::new();
                let mut chars = part.chars();
                while let Some(c) = chars.next() {
                    out.push(if c == '~' {
                        match chars.next()? {
                            '0' => '~',
                            '1' => '/',
                            _ => return None,
                        }
                    } else {
                        c
                    });
                }
                Some(out)
            })
            .collect(),
        _ => None,
    }
}

fn apply_patch(runtime: &mut Value, native: &str, patch: Patch) -> Option<()> {
    if !matches!(patch.op.as_str(), "add" | "remove" | "replace") {
        return None;
    }
    let path = path(patch.path)?;
    if path.is_empty() {
        if patch.op == "remove" {
            return None;
        }
        let state: Projection = serde_json::from_value(patch.value?).ok()?;
        if state.id != native || state.host_id != "local" {
            return None;
        }
        *runtime = Runtime::parse(&state.thread_runtime_status)?.value();
        return Some(());
    }
    if matches!(path[0].as_str(), "id" | "hostId") {
        return None;
    }
    if path[0] != "threadRuntimeStatus" {
        return Some(());
    }
    if path.len() == 1 {
        if patch.op == "remove" {
            return None;
        }
        *runtime = Runtime::parse(&patch.value?)?.value();
        return Some(());
    }
    // Retain only these tiny fields, and refuse unfamiliar status edits.
    if !matches!(path[1].as_str(), "type" | "activeFlags") || path.len() > 3 {
        return None;
    }
    if path.len() == 2 {
        let object = runtime.as_object_mut()?;
        match patch.op.as_str() {
            "remove" => {
                object.remove(&path[1])?;
            }
            "replace" if !object.contains_key(&path[1]) => return None,
            _ => {
                let value = patch.value?;
                if path[1] == "type" {
                    if !matches!(value.as_str(), Some("active" | "idle")) {
                        return None;
                    }
                } else if value.as_array().is_none_or(|v| {
                    v.len() > 2
                        || v.iter().any(|v| {
                            !matches!(v.as_str(), Some("waitingOnApproval" | "waitingOnUserInput"))
                        })
                }) {
                    return None;
                }
                object.insert(path[1].clone(), value);
            }
        }
    } else {
        if path[1] != "activeFlags" {
            return None;
        }
        let flags = runtime.get_mut("activeFlags")?.as_array_mut()?;
        let index = if path[2] == "-" && patch.op == "add" {
            flags.len()
        } else {
            let index = path[2].parse::<usize>().ok()?;
            if index.to_string() != path[2] {
                return None;
            }
            index
        };
        match patch.op.as_str() {
            "remove" if index < flags.len() => {
                flags.remove(index);
            }
            "add" if index <= flags.len() && flags.len() < 2 => {
                let value = patch.value?;
                validate_flag(&value)?;
                flags.insert(index, value);
            }
            "replace" if index < flags.len() => {
                let value = patch.value?;
                validate_flag(&value)?;
                flags[index] = value;
            }
            _ => return None,
        }
    }
    Some(())
}

fn validate_flag(value: &Value) -> Option<()> {
    matches!(
        value.as_str(),
        Some("waitingOnApproval" | "waitingOnUserInput")
    )
    .then_some(())
}
