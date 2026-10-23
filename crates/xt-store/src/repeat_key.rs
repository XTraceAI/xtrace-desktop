//! The single owner of M-20's grouping identity: the versioned opaque
//! comparison key of one canonical `tool_use` block ([`VERSION`] for most
//! tools, [`TOOL_FIELD_VERSION`] for Codex's command tools).
//!
//! M-20 groups the calls inside one hands-off stretch by `(tool name, first
//! identifying argument)`. The argument is transcript content, so it is read
//! here, in memory, at the same point [`crate::tool_use::classify`] derives the
//! structural columns — before content retention decides whether the block that
//! carried it survives. What is persisted is only this key: a SHA-256 digest
//! that two calls share exactly when they were the same call.
//!
//! **This is private local metadata, not anonymised or exportable data.** The
//! digest is a one-way projection of a command, path or pattern from the user's
//! own machine, and a caller holding a candidate argument can confirm it by
//! deriving the key again. It exists so the local metric can compare two calls
//! without keeping what they said. Nothing may publish it: it is absent from
//! the C-08 measurement projection and from every receipt, report and label,
//! and the digest is deliberately unreadable outside this crate so no
//! serialized shape can carry it by accident. Argument labels for the UI come
//! from the original source, transiently, when the user opens a detail view.
//!
//! Nothing here normalises. The tool name is the exact name stored beside the
//! call, and the argument is the exact string the block stated: no trimming, no
//! case folding, no path canonicalisation and no command parsing. Two calls
//! that differ by a space are two calls, because deciding they were the same
//! would require interpreting text this metric never reads.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// The base derivation: a call is identified by the first of
/// [`IDENTIFYING_FIELDS`] it states. Every tool not in [`TOOL_FIELDS`] is keyed
/// under it, with the preimage it has always had. Never change the preimage
/// under an unchanged version; a changed derivation gets a new version, and
/// keys of different versions are never the same call.
pub const VERSION: u32 = 1;

/// The per-tool derivation: a tool named in [`TOOL_FIELDS`] is identified by
/// that tool's own field instead. Codex states its command in fields M-20's
/// base list does not read (`exec`'s text, `exec_command`'s `cmd`, `js`'s
/// `code`), so under version 1 every call of those tools was the same call.
/// A record indexed under version 1 keeps its version-1 key until its source
/// is read again; that reading replaces the key (see `write.rs`).
pub const TOOL_FIELD_VERSION: u32 = 2;

/// Whether this build can compare keys of `version`: every version it
/// derives. A reader that meets another version treats the key as unknown.
pub fn readable(version: u32) -> bool {
    version == VERSION || version == TOOL_FIELD_VERSION
}

/// Codex's own command tools, each with the one field that states its
/// command. The pinned Codex reader wraps `exec`'s free-form text as
/// `{"input": text}`. No Claude built-in tool has these names, MCP tools are
/// prefixed, and the Cursor reader has never written them; `apply_patch` is
/// not here, because a patch is not a command.
pub const TOOL_FIELDS: [(&str, &str); 3] =
    [("exec", "input"), ("exec_command", "cmd"), ("js", "code")];

/// Domain label, hashed ahead of the version and the payload, so this digest
/// can never equal one another XTrace projection derives from similar values.
const DOMAIN: &str = "xtrace.metric.m20.tool-group";

/// The identifying-argument fields, in M-20's stated priority. The *first one
/// the observed input object contains* is the one that identifies the call,
/// whatever it holds; a later field never rescues an earlier unusable one.
pub const IDENTIFYING_FIELDS: [&str; 3] = ["command", "file_path", "pattern"];

/// One call's grouping identity. Equality is the whole of its meaning: two
/// blocks belong to the same M-20 group exactly when their keys are equal.
///
/// The digest is unreadable outside this crate on purpose — see the module
/// documentation — and `Debug` prints only the version for the same reason.
#[derive(Clone, PartialEq, Eq)]
pub struct GroupKey {
    version: u32,
    digest: String,
}

impl GroupKey {
    /// Which derivation produced this key.
    pub fn version(&self) -> u32 {
        self.version
    }

    pub(crate) fn digest(&self) -> &str {
        &self.digest
    }

    /// Adopt a stored `(group_version, group_key)` pair. Both columns are
    /// present together or the row states no key at all, which the schema also
    /// enforces; a version outside this build's integer range is no key it can
    /// read and is unknown rather than a key it might mistake for its own.
    pub(crate) fn stored(version: Option<i64>, digest: Option<String>) -> Option<Self> {
        Some(Self {
            version: u32::try_from(version?).ok()?,
            digest: digest?,
        })
    }
}

impl std::fmt::Debug for GroupKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GroupKey")
            .field("version", &self.version)
            .finish_non_exhaustive()
    }
}

/// What one observed `tool_use` input says about its identifying argument.
enum Argument<'a> {
    /// The first identifying field the object states, and its exact string.
    Stated { tag: &'static str, value: &'a str },
    /// An object that states none of the identifying fields. This is an
    /// observation, not a gap: the call really did name no such argument, and
    /// every call of the same tool that also names none is the same call.
    Absent,
    /// Nothing was observed that could identify the call: no input at all, an
    /// input that is not an object, or a selected field holding something other
    /// than a string. A key derived from a guess would group unlike calls.
    Unknown,
}

fn argument<'a>(input: Option<&'a Value>, identifying: &[&'static str]) -> Argument<'a> {
    let Some(Value::Object(fields)) = input else {
        return Argument::Unknown;
    };
    for &tag in identifying {
        if let Some(value) = fields.get(tag) {
            return match value.as_str() {
                Some(value) => Argument::Stated { tag, value },
                // The call stated this argument as something that is not a
                // string. Which call it was is unknown; the next field in the
                // priority describes a different argument of the same call.
                None => Argument::Unknown,
            };
        }
    }
    Argument::Absent
}

/// Derive the version-1 key of one call, or `None` when the observation cannot
/// state which call it was.
///
/// `name` is the tool name exactly as it is stored beside the call. `input` is
/// the block's own input value when the block carried one.
///
/// The preimage is one JSON array of fixed arity — domain, version, name, the
/// selected field's tag, the exact argument string — so it is unambiguous:
/// every part is a separately quoted and escaped element, and no part can spell
/// a delimiter that fuses it with the next. An object stating none of the
/// identifying fields uses `null` in both argument slots, which no tag can
/// collide with, so "this call named no identifying argument" is itself an
/// explicit, comparable answer.
pub fn derive(name: &str, input: Option<&Value>) -> Option<GroupKey> {
    let version = if TOOL_FIELDS.iter().any(|(tool, _)| *tool == name) {
        TOOL_FIELD_VERSION
    } else {
        VERSION
    };
    derive_under(version, name, input)
}

/// [`derive`] under one stated version: version 1 reads
/// [`IDENTIFYING_FIELDS`]; version 2 reads the tool's own field from
/// [`TOOL_FIELDS`] and derives nothing for any other tool.
fn derive_under(version: u32, name: &str, input: Option<&Value>) -> Option<GroupKey> {
    let own;
    let identifying: &[&'static str] = match version {
        VERSION => &IDENTIFYING_FIELDS,
        TOOL_FIELD_VERSION => {
            own = [TOOL_FIELDS.iter().find(|(tool, _)| *tool == name)?.1];
            &own
        }
        _ => return None,
    };
    let (tag, value) = match argument(input, identifying) {
        Argument::Unknown => return None,
        Argument::Absent => (None, None),
        Argument::Stated { tag, value } => (Some(tag), Some(value)),
    };
    let preimage = serde_json::to_vec(&(DOMAIN, version, name, tag, value))
        .expect("strings and integers are serializable");
    Some(GroupKey {
        version,
        digest: format!("{:x}", Sha256::digest(preimage)),
    })
}

/// Whether a stored `(group_version, group_key)` is the key every call of
/// `name` that names no identifying argument shares: under its version, the
/// call stated none of the fields that version reads, so M-20 treats all such
/// calls of that tool as one. Works for a key of either version, so an older
/// record's version-1 key is recognised too. Answered from the stored key
/// alone; no argument is read.
pub fn names_no_argument(name: &str, version: i64, digest: &str) -> bool {
    u32::try_from(version).is_ok_and(|version| {
        derive_under(version, name, Some(&Value::Object(Default::default())))
            .is_some_and(|key| key.digest == digest)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeSet;

    fn key(name: &str, input: Value) -> Option<String> {
        derive(name, Some(&input)).map(|key| key.digest().to_owned())
    }

    #[test]
    fn a_call_naming_no_identifying_argument_is_recognised_from_its_key() {
        let none = key("navigate", json!({"url": "a"})).unwrap();
        let other = key("navigate", json!({"url": "b"})).unwrap();
        assert_eq!(none, other);
        assert!(names_no_argument("navigate", 1, &none));
        let stated = key("Bash", json!({"command": "ls"})).unwrap();
        assert!(!names_no_argument("Bash", 1, &stated));
        // The same key under another tool's name, or another version, is not it.
        assert!(!names_no_argument("js", 1, &none));
        assert!(!names_no_argument("navigate", 2, &none));
        // Either version of a Codex tool: the version-1 key an older build
        // stored, and a version-2 key whose own field is absent.
        assert!(names_no_argument("exec", 1, PINNED_EXEC_V1));
        let absent = derive("exec", Some(&json!({"other": 1}))).unwrap();
        assert_eq!(absent.version(), TOOL_FIELD_VERSION);
        assert!(names_no_argument("exec", 2, absent.digest()));
        let stated = key("exec", json!({"input": "ls"})).unwrap();
        assert!(!names_no_argument("exec", 2, &stated));
    }

    /// Version-1 digests as the build before the per-tool fields derived
    /// them (8eb3eee7), and as the live index stores them.
    const PINNED_EXEC_V1: &str = "22f2a1b254303b3a67faea020a59512fc845e302b6a176066165b696050ac2b0";

    #[test]
    fn version_one_keys_are_byte_identical_to_the_earlier_build() {
        for (name, input, digest) in [
            (
                "Bash",
                json!({"command": "ls -la"}),
                "de77532ef0e26c6c72c627bcc3c7b7580c2e6cba8bb89368d0d97daf92e4a6d9",
            ),
            (
                "Edit",
                json!({"file_path": "/a.rs", "old_string": "x"}),
                "bce79d667208fde804158a5c7479a4fed067a239b5bfc4d4794614c56b530cc0",
            ),
            (
                "Grep",
                json!({"pattern": "fn main", "glob": "*.rs"}),
                "1159096ad32f858251fff3e197ada868d336931e2bdf842f447cef2d755b00a2",
            ),
            (
                "TodoWrite",
                json!({"todos": []}),
                "f523340b4778b58579df649b4c5a4d9cfd0143a6be02391ca2fdef7bc4dbc188",
            ),
            // A patch is not a command: apply_patch keeps its version-1 key.
            (
                "apply_patch",
                json!({"input": "*** Begin Patch"}),
                "aca36f7b4648bb8813a738e3b70e17ddc77410f43f74c9dab7c64531ed93b198",
            ),
        ] {
            let derived = derive(name, Some(&input)).unwrap();
            assert_eq!(
                (derived.version(), derived.digest()),
                (VERSION, digest),
                "{name}"
            );
        }
        // What an older build stored for every Codex exec call.
        let old = derive_under(VERSION, "exec", Some(&json!({"input": "ls"}))).unwrap();
        assert_eq!(old.digest(), PINNED_EXEC_V1);
    }

    #[test]
    fn codex_command_tools_are_keyed_by_their_own_field() {
        for (name, field) in TOOL_FIELDS {
            let call = |value: &str| {
                let mut input = serde_json::Map::new();
                input.insert(field.to_owned(), json!(value));
                derive(name, Some(&Value::Object(input))).unwrap()
            };
            assert_eq!(call("cargo test").version(), TOOL_FIELD_VERSION, "{name}");
            assert_eq!(call("cargo test"), call("cargo test"), "{name}");
            assert_ne!(call("cargo test"), call("cargo build"), "{name}");
            // Never the same call as its version-1 key, or another tool's.
            let v1 = derive_under(VERSION, name, Some(&json!({field: "cargo test"}))).unwrap();
            assert_ne!(call("cargo test"), v1, "{name}");
            assert_ne!(call("cargo test").digest(), v1.digest(), "{name}");
        }
        assert_ne!(
            key("exec", json!({"input": "ls"})),
            key("js", json!({"code": "ls"}))
        );
        // The base fields are not read for these tools, and a non-string
        // command is unknown.
        assert_eq!(
            key("exec_command", json!({"cmd": "ls", "command": "a"})),
            key("exec_command", json!({"cmd": "ls", "command": "b"}))
        );
        assert_eq!(key("exec_command", json!({"cmd": ["ls"]})), None);
        assert!(readable(VERSION) && readable(TOOL_FIELD_VERSION) && !readable(3));
    }

    #[test]
    fn the_first_field_the_object_states_identifies_the_call() {
        // Priority decides between fields that are both present, and the
        // absent ones contribute nothing: naming only `pattern` derives the
        // same key as naming it beside no earlier field.
        let command = key(
            "Bash",
            json!({"command":"ls","file_path":"a","pattern":"b"}),
        );
        assert_eq!(command, key("Bash", json!({"command":"ls"})));
        assert_eq!(
            key("Edit", json!({"file_path":"a","pattern":"b"})),
            key("Edit", json!({"file_path":"a"}))
        );
        // Unrelated fields are not read at all.
        assert_eq!(
            key(
                "Grep",
                json!({"pattern":"p","glob":"*.rs","output_mode":"content"})
            ),
            key("Grep", json!({"pattern":"p"}))
        );
        // A different field holding the same string is a different call.
        assert_ne!(
            key("Same", json!({"command":"x"})),
            key("Same", json!({"file_path":"x"}))
        );
        assert_ne!(
            key("Same", json!({"file_path":"x"})),
            key("Same", json!({"pattern":"x"}))
        );
    }

    #[test]
    fn the_selected_field_is_never_abandoned_for_a_later_one() {
        // `command` is present, so it is the identifying argument. It states
        // something that is not a string, so which call this was is unknown —
        // reading `file_path` instead would answer about another argument.
        for stated in [json!(null), json!(7), json!(["ls"]), json!({"argv":"ls"})] {
            assert_eq!(
                key(
                    "Bash",
                    json!({"command":stated,"file_path":"a","pattern":"b"})
                ),
                None,
                "{stated}"
            );
        }
        assert_eq!(key("Edit", json!({"file_path":42,"pattern":"p"})), None);
    }

    #[test]
    fn an_object_naming_none_of_the_fields_is_an_answer_and_a_missing_input_is_not() {
        // Observed, and comparable: two such calls of one tool are one call.
        let none = key("TodoWrite", json!({}));
        assert!(none.is_some());
        assert_eq!(none, key("TodoWrite", json!({"todos":[],"other":1})));
        // ... and not the same call as any stated argument, nor as the same
        // marker under another tool.
        assert_ne!(none, key("TodoWrite", json!({"command":""})));
        assert_ne!(none, key("Other", json!({})));
        // Nothing observed at all stays unknown.
        assert_eq!(derive("Bash", None), None);
        for input in [json!("ls"), json!(7), json!(null), json!(["command"])] {
            assert_eq!(key("Bash", input.clone()), None, "{input}");
        }
    }

    #[test]
    fn nothing_is_normalised_and_no_part_can_spell_a_delimiter() {
        // Whitespace, case and path or command shape are all preserved: these
        // are seven different calls, not one.
        let distinct: BTreeSet<_> = [
            key("Bash", json!({"command":"ls -la"})),
            key("Bash", json!({"command":"ls  -la"})),
            key("Bash", json!({"command":" ls -la"})),
            key("Bash", json!({"command":"ls -la "})),
            key("Bash", json!({"command":"LS -la"})),
            key("Bash", json!({"command":"ls -la; true"})),
            key(" Bash", json!({"command":"ls -la"})),
        ]
        .into_iter()
        .collect();
        assert_eq!(distinct.len(), 7);
        // Payload parts cannot be shifted across the boundary between them,
        // whichever separator a naive encoding might have used.
        for (left, right) in [
            (
                key("A|B", json!({"command":"C"})),
                key("A", json!({"command":"B|C"})),
            ),
            (
                key("A\u{0}B", json!({"command":"C"})),
                key("A", json!({"command":"B\u{0}C"})),
            ),
            (
                key("A", json!({"command":"B\",\"file_path\":\"C"})),
                key("A", json!({"command":"B","file_path":"C"})),
            ),
            (
                key("command", json!({"command":"file_path"})),
                key("file_path", json!({"file_path":"command"})),
            ),
        ] {
            assert!(left.is_some());
            assert_ne!(left, right);
        }
    }

    #[test]
    fn version_one_keys_are_fixed_opaque_hex_that_a_later_build_must_reproduce() {
        let key = derive("Edit", Some(&json!({"file_path":"/tmp/a.rs"}))).unwrap();
        assert_eq!(key.version(), VERSION);
        // Pinned so the derivation cannot change without changing VERSION.
        assert_eq!(
            key.digest(),
            "66594b0170a51bae8c5bae7a9cd671b0899455e2f14f838785d83e64da5c0448"
        );
        assert_eq!(
            derive("TodoWrite", Some(&json!({}))).unwrap().digest(),
            "f523340b4778b58579df649b4c5a4d9cfd0143a6be02391ca2fdef7bc4dbc188"
        );
        assert!(
            key.digest().len() == 64
                && key
                    .digest()
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        );
        // Debug never carries the digest.
        assert_eq!(format!("{key:?}"), "GroupKey { version: 1, .. }");
    }

    #[test]
    fn stored_pairs_round_trip_and_a_half_stated_row_states_no_key() {
        let key = derive("Read", Some(&json!({"file_path":"a"}))).unwrap();
        assert_eq!(
            GroupKey::stored(Some(i64::from(VERSION)), Some(key.digest().to_owned())),
            Some(key.clone())
        );
        // A future version's key is read back as that version's, so a reader
        // can tell "not mine" from "not there".
        assert_eq!(
            GroupKey::stored(Some(9), Some(key.digest().to_owned()))
                .unwrap()
                .version(),
            9
        );
        for (version, digest) in [
            (None, Some(key.digest().to_owned())),
            (Some(1), None),
            (None, None),
            (Some(-1), Some(key.digest().to_owned())),
        ] {
            assert_eq!(GroupKey::stored(version, digest), None);
        }
    }
}
