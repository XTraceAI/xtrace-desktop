//! Status-only projection of one IPC frame body, fed in arbitrary chunks.
//!
//! Modeled on the compaction scanner in xt-ingest
//! (`native/session_compactions/jsonl.rs`): every byte is validated as JSON
//! and discarded unless it sits under a small whitelist of keys, and only that
//! projection is handed to serde_json. A snapshot carries the whole
//! conversation (17MB observed), but status needs a few short fields.
//!
//! That scanner pulls from a blocking reader. This one is push-based and can
//! stop at any byte, because the socket worker must keep servicing leases,
//! releases and shutdown while a large frame is still arriving.
use super::protocol;
use std::collections::BTreeSet;

/// The retained projection (kept values plus inspected key names). 256
/// bounded patches with long paths fit comfortably; no transcript does.
pub(super) const MAX_PROJECTION: usize = 1024 * 1024;
const KEY: usize = 256;
const NUMBER: usize = 128;
const MAX_DEPTH: usize = 128;
/// Longest raw string kept in a patch value whose path is not known yet.
/// Every string a status path could accept there is a short exact word
/// ("waitingOnUserInput" is the longest at 18 characters, 108 bytes fully
/// \u-escaped) or a conversation id that must equal a 36-character id.
const SHORT_TEXT: usize = 128;

#[derive(Debug, PartialEq, Eq)]
pub(super) struct Invalid;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Shape {
    Root,
    Result,
    Params,
    Change,
    State,
    Status,
    Patches,
    Patch,
    /// An object patch value seen before its path. A later path could be the
    /// whole state (reads id, hostId, threadRuntimeStatus) or
    /// `threadRuntimeStatus` (reads type, activeFlags); other keys drop.
    PatchValue,
    /// `threadRuntimeStatus` inside such a value: type and activeFlags.
    PreStatus,
    /// An array in such a value. Only activeFlags reads an array, and only
    /// as at most two short strings; every status path rejects any other
    /// array exactly as it rejects null. Such an array is kept while it can
    /// still be valid flags, and is otherwise replaced by `null`.
    Flags,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Keep {
    Drop,
    Verbatim,
    Project(Shape),
    /// Inside a patch value whose path is not known yet: a string of at most
    /// SHORT_TEXT raw bytes, a number, true, false or null is kept; a longer
    /// string, an object or an array is replaced by `null`. Every status
    /// path that could read such a position needs a short exact string, and
    /// rejects null exactly where it rejects the replaced value.
    Short,
}

/// What a patch's own path says about its value, once the path is known.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PatchPath {
    Unseen,
    Root,
    Status,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Capture {
    None,
    ConversationId,
    PatchPath,
}

/// Exactly the fields `protocol::Message` and `ThreadStatus` read.
fn rule(shape: Shape, key: &str, patch: PatchPath) -> Keep {
    use Keep::{Drop, Project, Verbatim};
    match (shape, key) {
        (
            Shape::Root,
            "type" | "requestId" | "sourceClientId" | "handledByClientId" | "version" | "method"
            | "resultType" | "clientId" | "status",
        ) => Verbatim,
        (Shape::Root, "result") => Project(Shape::Result),
        (Shape::Root, "params") => Project(Shape::Params),
        (Shape::Result, "clientId") => Verbatim,
        (Shape::Params, "conversationId" | "hostId" | "clientId" | "status" | "connected") => {
            Verbatim
        }
        (Shape::Params, "change") => Project(Shape::Change),
        (Shape::Change, "type" | "revision" | "baseRevision") => Verbatim,
        (Shape::Change, "conversationState") => Project(Shape::State),
        (Shape::Change, "patches") => Project(Shape::Patches),
        (Shape::State, "id" | "hostId")
        | (Shape::Status, "type" | "activeFlags")
        | (Shape::Patch, "op" | "path") => Verbatim,
        (Shape::State, "threadRuntimeStatus") => Project(Shape::Status),
        (Shape::PatchValue, "id" | "hostId" | "type") | (Shape::PreStatus, "type") => Keep::Short,
        (Shape::PatchValue | Shape::PreStatus, "activeFlags") => Project(Shape::Flags),
        (Shape::PatchValue, "threadRuntimeStatus") => Project(Shape::PreStatus),
        // apply_patch never reads a value whose path is invalid, an identity
        // path, or outside threadRuntimeStatus; those values can be dropped.
        (Shape::Patch, "value") => match patch {
            PatchPath::Unseen => Project(Shape::PatchValue),
            PatchPath::Root => Project(Shape::State),
            PatchPath::Status => Project(Shape::Status),
            PatchPath::Other => Drop,
        },
        _ => Drop,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Expect {
    /// Object: first key or `}`. Array: first element or `]`.
    First,
    Key,
    Colon,
    Value,
    /// `,` or the closing bracket.
    After,
}

struct Container {
    array: bool,
    keep: bool,
    shape: Option<Shape>,
    expect: Expect,
    emitted: bool,
    seen: BTreeSet<String>,
    next: Keep,
    capture: Capture,
    patch: PatchPath,
    /// Output offset of a Flags `[`, entries kept, and whether this
    /// container was replaced by null.
    start: usize,
    items: u8,
    replaced: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Escape {
    None,
    Backslash,
    Hex {
        left: u8,
        value: u16,
        low: bool,
    },
    /// A high surrogate needs `\u` and a low surrogate next.
    HighSurrogate,
    HighBackslash,
    Utf8 {
        bytes: [u8; 4],
        have: u8,
        need: u8,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Number {
    Minus,
    Zero,
    Int,
    Dot,
    Fraction,
    Exponent,
    ExponentSign,
    ExponentInt,
}

enum Token {
    Idle,
    Text {
        keep: bool,
        key: bool,
        length: usize,
        escape: Escape,
        /// Output offset of the opening quote for a `Keep::Short` string.
        short: Option<usize>,
        replaced: bool,
    },
    Literal {
        word: &'static [u8],
        at: usize,
        keep: bool,
    },
    Number {
        state: Number,
        keep: bool,
    },
}

pub(super) struct Scanner {
    stack: Vec<Container>,
    token: Token,
    started: bool,
    done: bool,
    output: Vec<u8>,
    key: Vec<u8>,
    number: Vec<u8>,
    inspected: usize,
    /// The current key was too long to be kept and is being skipped.
    key_dropped: bool,
    capture: Option<(Capture, usize, usize)>,
    conversation: Option<String>,
}

impl Default for Scanner {
    fn default() -> Self {
        Self {
            stack: Vec::new(),
            token: Token::Idle,
            started: false,
            done: false,
            output: Vec::new(),
            key: Vec::new(),
            number: Vec::new(),
            inspected: 0,
            key_dropped: false,
            capture: None,
            conversation: None,
        }
    }
}

impl Scanner {
    /// `params.conversationId` once that value has been read, so a frame that
    /// later fails can still be attributed to one conversation.
    pub fn conversation(&self) -> Option<&str> {
        self.conversation.as_deref()
    }

    /// Bytes currently retained for this frame, for bound assertions.
    #[cfg(test)]
    pub fn retained(&self) -> usize {
        self.output.capacity() + self.key.capacity() + self.number.capacity()
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), Invalid> {
        let mut at = 0;
        while at < bytes.len() {
            at += self.step(&bytes[at..])?;
        }
        Ok(())
    }

    /// The whole projection as compact JSON, after the last body byte.
    pub fn finish(self) -> Result<Vec<u8>, Invalid> {
        if self.done && matches!(self.token, Token::Idle) {
            Ok(self.output)
        } else {
            Err(Invalid)
        }
    }

    fn put(&mut self, bytes: &[u8]) -> Result<(), Invalid> {
        if self.output.len().saturating_add(bytes.len()) > MAX_PROJECTION {
            return Err(Invalid);
        }
        self.output.extend_from_slice(bytes);
        Ok(())
    }

    fn text(&mut self, bytes: &[u8], keep: bool, key: bool) -> Result<(), Invalid> {
        if !keep {
            Ok(())
        } else if key {
            self.key.extend_from_slice(bytes);
            Ok(())
        } else {
            self.put(bytes)
        }
    }

    /// Consumes at least one byte of `bytes` and returns how many.
    fn step(&mut self, bytes: &[u8]) -> Result<usize, Invalid> {
        let byte = bytes[0];
        match self.token {
            Token::Idle => self.structural(byte).map(|()| 1),
            Token::Text {
                keep,
                key,
                length,
                escape,
                short,
                replaced,
            } => self.string(bytes, (keep, key, short, replaced), length, escape),
            Token::Literal { word, at, keep } => {
                if word[at] != byte {
                    return Err(Invalid);
                }
                if at + 1 == word.len() {
                    self.token = Token::Idle;
                    if keep {
                        self.put(word)?;
                    }
                    self.value_done()?;
                } else {
                    self.token = Token::Literal {
                        word,
                        at: at + 1,
                        keep,
                    };
                }
                Ok(1)
            }
            Token::Number { state, keep } => {
                let next = match (state, byte) {
                    (Number::Minus, b'0') => Some(Number::Zero),
                    (Number::Minus, b'1'..=b'9') => Some(Number::Int),
                    (Number::Int, b'0'..=b'9') => Some(Number::Int),
                    (Number::Zero | Number::Int, b'.') => Some(Number::Dot),
                    (Number::Dot | Number::Fraction, b'0'..=b'9') => Some(Number::Fraction),
                    (Number::Zero | Number::Int | Number::Fraction, b'e' | b'E') => {
                        Some(Number::Exponent)
                    }
                    (Number::Exponent, b'+' | b'-') => Some(Number::ExponentSign),
                    (
                        Number::Exponent | Number::ExponentSign | Number::ExponentInt,
                        b'0'..=b'9',
                    ) => Some(Number::ExponentInt),
                    _ => None,
                };
                if let Some(state) = next {
                    if self.number.len() == NUMBER {
                        return Err(Invalid);
                    }
                    self.number.push(byte);
                    self.token = Token::Number { state, keep };
                    return Ok(1);
                }
                if !matches!(
                    state,
                    Number::Zero | Number::Int | Number::Fraction | Number::ExponentInt
                ) {
                    return Err(Invalid);
                }
                // The terminator belongs to the enclosing structure.
                self.token = Token::Idle;
                if keep {
                    let number = std::mem::take(&mut self.number);
                    self.put(&number)?;
                    self.number = number;
                }
                self.number.clear();
                self.value_done()?;
                Ok(0)
            }
        }
    }

    fn string(
        &mut self,
        bytes: &[u8],
        (mut keep, key, short, mut replaced): (bool, bool, Option<usize>, bool),
        mut length: usize,
        escape: Escape,
    ) -> Result<usize, Invalid> {
        let byte = bytes[0];
        let mut used = 1;
        let next = match escape {
            Escape::None => match byte {
                b'"' => {
                    self.token = Token::Idle;
                    self.text(b"\"", keep, key)?;
                    if replaced {
                        self.put(b"null")?;
                    }
                    return if key {
                        self.key_done().map(|()| 1)
                    } else {
                        self.value_done().map(|()| 1)
                    };
                }
                b'\\' => {
                    self.text(b"\\", keep, key)?;
                    Escape::Backslash
                }
                0..=31 => return Err(Invalid),
                128..=255 => {
                    let need = match byte {
                        0xc2..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf4 => 4,
                        _ => return Err(Invalid),
                    };
                    let mut buffer = [0; 4];
                    buffer[0] = byte;
                    Escape::Utf8 {
                        bytes: buffer,
                        have: 1,
                        need,
                    }
                }
                _ => {
                    // The common, potentially enormous ASCII run is handled a
                    // chunk at a time, with no per-byte state transitions.
                    used = bytes
                        .iter()
                        .position(|b| *b < 32 || *b >= 128 || matches!(*b, b'"' | b'\\'))
                        .unwrap_or(bytes.len());
                    // Decide before copying: one run can be a whole read.
                    if let Some(start) = short
                        && keep
                        && length + used > SHORT_TEXT
                    {
                        self.output.truncate(start);
                        (keep, replaced) = (false, true);
                    }
                    self.text(&bytes[..used], keep, key)?;
                    Escape::None
                }
            },
            Escape::Backslash => match byte {
                b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {
                    self.text(&[byte], keep, key)?;
                    Escape::None
                }
                b'u' => {
                    self.text(b"u", keep, key)?;
                    Escape::Hex {
                        left: 4,
                        value: 0,
                        low: false,
                    }
                }
                _ => return Err(Invalid),
            },
            Escape::Hex { left, value, low } => {
                let digit = (byte as char).to_digit(16).ok_or(Invalid)? as u16;
                self.text(&[byte], keep, key)?;
                let value = value * 16 + digit;
                if left > 1 {
                    Escape::Hex {
                        left: left - 1,
                        value,
                        low,
                    }
                } else if low {
                    if !(0xdc00..=0xdfff).contains(&value) {
                        return Err(Invalid);
                    }
                    Escape::None
                } else if (0xd800..=0xdbff).contains(&value) {
                    Escape::HighSurrogate
                } else if (0xdc00..=0xdfff).contains(&value) {
                    return Err(Invalid);
                } else {
                    Escape::None
                }
            }
            Escape::HighSurrogate if byte == b'\\' => {
                self.text(b"\\", keep, key)?;
                Escape::HighBackslash
            }
            Escape::HighBackslash if byte == b'u' => {
                self.text(b"u", keep, key)?;
                Escape::Hex {
                    left: 4,
                    value: 0,
                    low: true,
                }
            }
            Escape::HighSurrogate | Escape::HighBackslash => return Err(Invalid),
            Escape::Utf8 {
                bytes: mut buffer,
                have,
                need,
            } => {
                buffer[have as usize] = byte;
                let have = have + 1;
                if have < need {
                    Escape::Utf8 {
                        bytes: buffer,
                        have,
                        need,
                    }
                } else {
                    // Rejects overlong forms, surrogates and > U+10FFFF,
                    // also when the codepoint spans two reads.
                    let code = &buffer[..need as usize];
                    std::str::from_utf8(code).map_err(|_| Invalid)?;
                    self.text(code, keep, key)?;
                    Escape::None
                }
            }
        };
        length += used;
        if key && keep && length > KEY {
            // Such a key cannot be kept; a patch value before its path skips
            // it, while every other inspected object refuses the frame.
            if !self
                .stack
                .last()
                .is_some_and(|top| matches!(top.shape, Some(Shape::PatchValue | Shape::PreStatus)))
            {
                return Err(Invalid);
            }
            self.key.clear();
            (keep, self.key_dropped) = (false, true);
        }
        if let Some(start) = short
            && keep
            && length > SHORT_TEXT
        {
            self.output.truncate(start);
            (keep, replaced) = (false, true);
        }
        self.token = Token::Text {
            keep,
            key,
            length,
            escape: next,
            short,
            replaced,
        };
        Ok(used)
    }

    fn structural(&mut self, byte: u8) -> Result<(), Invalid> {
        if matches!(byte, b' ' | b'\t' | b'\n' | b'\r') {
            return Ok(());
        }
        if self.done {
            return Err(Invalid);
        }
        let Some(top) = self.stack.last_mut() else {
            if self.started || byte != b'{' {
                return Err(Invalid);
            }
            self.started = true;
            return self.value(byte, Keep::Project(Shape::Root));
        };
        match (top.array, top.expect, byte) {
            (false, Expect::First, b'}') | (false, Expect::After, b'}') => self.close(b"}"),
            (true, Expect::First, b']') | (true, Expect::After, b']') => self.close(b"]"),
            (false, Expect::First | Expect::Key, b'"') => {
                self.key.clear();
                self.key_dropped = false;
                let keep = top.keep;
                self.token = Token::Text {
                    keep,
                    key: true,
                    length: 0,
                    escape: Escape::None,
                    short: None,
                    replaced: false,
                };
                self.text(b"\"", keep, true)
            }
            (false, Expect::Colon, b':') => {
                top.expect = Expect::Value;
                Ok(())
            }
            (false, Expect::After, b',') => {
                top.expect = Expect::Key;
                Ok(())
            }
            (true, Expect::After, b',') => {
                top.expect = Expect::Value;
                Ok(())
            }
            (false, Expect::Value, _) => {
                let decision = top.next;
                let capture = std::mem::replace(&mut top.capture, Capture::None);
                if capture != Capture::None {
                    self.capture = Some((capture, self.output.len(), self.stack.len()));
                }
                self.value(byte, decision)
            }
            (true, Expect::First | Expect::Value, _) => {
                if top.keep && top.shape == Some(Shape::Flags) {
                    if byte != b'"' || top.items >= 2 {
                        self.replace_flags();
                    } else {
                        top.items += 1;
                    }
                }
                let Some(top) = self.stack.last_mut() else {
                    return Err(Invalid);
                };
                let decision = if !top.keep {
                    Keep::Drop
                } else if top.shape == Some(Shape::Patches) {
                    Keep::Project(Shape::Patch)
                } else if top.shape == Some(Shape::Flags) {
                    Keep::Short
                } else {
                    Keep::Verbatim
                };
                let comma = top.keep && top.emitted;
                top.emitted |= top.keep;
                top.expect = Expect::Value;
                if comma {
                    self.put(b",")?;
                }
                self.value(byte, decision)
            }
            _ => Err(Invalid),
        }
    }

    fn value(&mut self, byte: u8, decision: Keep) -> Result<(), Invalid> {
        let keep = decision != Keep::Drop;
        // Any position in a patch value whose path is not known yet.
        let short = matches!(
            decision,
            Keep::Short | Keep::Project(Shape::PatchValue | Shape::PreStatus | Shape::Flags)
        );
        match byte {
            b'{' | b'[' => {
                if self.stack.len() >= MAX_DEPTH {
                    return Err(Invalid);
                }
                let array = byte == b'[';
                let shape = match (decision, array) {
                    (Keep::Project(Shape::Patches), true) => Some(Shape::Patches),
                    (Keep::Project(Shape::PatchValue | Shape::Flags), true) => Some(Shape::Flags),
                    (Keep::Project(Shape::Patches | Shape::Flags), false) => None,
                    (Keep::Project(shape), false) => Some(shape),
                    _ => None,
                };
                // A container no status path could accept here: skip its
                // contents and emit null when it closes.
                let replaced = short && shape.is_none();
                let keep = keep && !replaced;
                self.stack.push(Container {
                    array,
                    keep,
                    shape,
                    expect: Expect::First,
                    emitted: false,
                    seen: BTreeSet::new(),
                    next: Keep::Drop,
                    capture: Capture::None,
                    patch: PatchPath::Unseen,
                    start: self.output.len(),
                    items: 0,
                    replaced,
                });
                if keep {
                    self.put(&[byte])?;
                }
                Ok(())
            }
            b'"' => {
                self.token = Token::Text {
                    keep,
                    key: false,
                    length: 0,
                    escape: Escape::None,
                    short: short.then_some(self.output.len()),
                    replaced: false,
                };
                self.text(b"\"", keep, false)
            }
            b't' | b'f' | b'n' => {
                let word: &'static [u8] = match byte {
                    b't' => b"true",
                    b'f' => b"false",
                    _ => b"null",
                };
                self.token = Token::Literal { word, at: 1, keep };
                Ok(())
            }
            b'-' | b'0'..=b'9' => {
                self.number.clear();
                self.number.push(byte);
                let state = match byte {
                    b'-' => Number::Minus,
                    b'0' => Number::Zero,
                    _ => Number::Int,
                };
                self.token = Token::Number { state, keep };
                Ok(())
            }
            _ => Err(Invalid),
        }
    }

    fn key_done(&mut self) -> Result<(), Invalid> {
        let Some(top) = self.stack.last_mut() else {
            return Err(Invalid);
        };
        top.expect = Expect::Colon;
        top.capture = Capture::None;
        if !top.keep || self.key_dropped {
            top.next = Keep::Drop;
            return Ok(());
        }
        let key: String = serde_json::from_slice(&self.key).map_err(|_| Invalid)?;
        let decision = match top.shape {
            Some(shape) => rule(shape, &key, top.patch),
            None => Keep::Verbatim,
        };
        // A patch value before its path may be any object; its dropped keys
        // are neither retained nor charged, so its size cannot skip a frame.
        if decision == Keep::Drop && matches!(top.shape, Some(Shape::PatchValue | Shape::PreStatus))
        {
            top.next = Keep::Drop;
            return Ok(());
        }
        self.inspected = self.inspected.saturating_add(self.key.len());
        if self.inspected > MAX_PROJECTION {
            return Err(Invalid);
        }
        if !top.seen.insert(key.clone()) {
            return Err(Invalid);
        }
        top.next = decision;
        top.capture = match (top.shape, key.as_str()) {
            (Some(Shape::Params), "conversationId") => Capture::ConversationId,
            (Some(Shape::Patch), "path") => Capture::PatchPath,
            _ => Capture::None,
        };
        if decision == Keep::Drop {
            return Ok(());
        }
        let comma = top.emitted;
        top.emitted = true;
        if comma {
            self.put(b",")?;
        }
        let encoded = serde_json::to_vec(&key).map_err(|_| Invalid)?;
        self.put(&encoded)?;
        self.put(b":")
    }

    /// This Flags array cannot be valid flags: drop what it kept so far and
    /// everything after, and emit `null` in its place when it closes.
    fn replace_flags(&mut self) {
        if let Some(top) = self.stack.last_mut() {
            self.output.truncate(top.start);
            top.keep = false;
            top.replaced = true;
        }
    }

    fn close(&mut self, bracket: &[u8]) -> Result<(), Invalid> {
        let container = self.stack.pop().ok_or(Invalid)?;
        if container.keep {
            self.put(bracket)?;
        } else if container.replaced {
            self.put(b"null")?;
        }
        self.value_done()
    }

    fn value_done(&mut self) -> Result<(), Invalid> {
        let depth = self.stack.len();
        let Some(top) = self.stack.last_mut() else {
            self.done = true;
            return Ok(());
        };
        top.expect = Expect::After;
        if let Some((kind, start, at)) = self.capture
            && at == depth
        {
            self.capture = None;
            let value = &self.output[start..];
            match kind {
                Capture::ConversationId => {
                    self.conversation = serde_json::from_slice::<String>(value).ok();
                }
                Capture::PatchPath => {
                    top.patch = serde_json::from_slice(value)
                        .ok()
                        .and_then(protocol::path)
                        .map_or(PatchPath::Other, |path| match path.first() {
                            None => PatchPath::Root,
                            Some(first) if first == "threadRuntimeStatus" => PatchPath::Status,
                            Some(_) => PatchPath::Other,
                        });
                }
                Capture::None => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn project(bytes: &[u8], chunk: usize) -> Result<Value, Invalid> {
        let mut scanner = Scanner::default();
        for part in bytes.chunks(chunk.max(1)) {
            scanner.feed(part)?;
        }
        Ok(serde_json::from_slice(&scanner.finish()?).unwrap())
    }

    #[test]
    fn live_projection_keeps_only_status_fields_at_every_chunk_size() {
        let body = json!({"type":"broadcast","sourceClientId":"owner","version":11,
            "method":"thread-stream-state-changed","targetClientIds":["me"],
            "params":{"conversationId":"c","hostId":"local","extra":{"a":[1,2]},
                "change":{"type":"snapshot","revision":7,"conversationState":{
                    "turns":[{"id":"turn","text":"private \u{e9}\u{1F600} \"q\" \\ \n",
                        "n":[-1.5e-3,0,12,true,false,null]}],
                    "id":"c","hostId":"local","title":"private",
                    "threadRuntimeStatus":{"type":"active","activeFlags":["waitingOnApproval"],"x":"y"}}}}});
        let expected = json!({"type":"broadcast","sourceClientId":"owner","version":11,
            "method":"thread-stream-state-changed",
            "params":{"conversationId":"c","hostId":"local",
                "change":{"type":"snapshot","revision":7,"conversationState":{
                    "id":"c","hostId":"local",
                    "threadRuntimeStatus":{"type":"active","activeFlags":["waitingOnApproval"]}}}}});
        let bytes = serde_json::to_vec_pretty(&body).unwrap();
        for chunk in [1, 2, 3, 7, 64, bytes.len()] {
            assert_eq!(project(&bytes, chunk).unwrap(), expected, "chunk {chunk}");
        }
        // Escaped surrogate pairs and raw UTF-8 split across every boundary.
        let raw = br#"{"type":"x","skip":"\uD83D\uDE00 \u00e9","clientId":"\uD83D\uDE00\u00e9"}"#;
        for chunk in 1..8 {
            assert_eq!(
                project(raw, chunk).unwrap(),
                json!({"type":"x","clientId":"\u{1F600}\u{e9}"})
            );
        }
    }

    #[test]
    fn live_projection_patch_values_follow_their_own_paths() {
        let body = json!({"type":"broadcast","params":{"change":{"type":"patches",
            "baseRevision":1,"revision":2,"patches":[
                {"op":"add","path":["turns",0],"value":{"text":"private","id":"x"}},
                {"op":"replace","path":"/threadRuntimeStatus/activeFlags","value":["waitingOnUserInput"]},
                {"op":"replace","path":[],"value":{"id":"c","hostId":"local","turns":[1],
                    "threadRuntimeStatus":{"type":"idle"}}},
                {"value":{"type":"active","activeFlags":[],"turns":["private"]},"op":"replace",
                    "path":["threadRuntimeStatus"]},
                {"op":"replace","path":"/threadRuntimeStatus/type","value":"active"}]}}});
        let expected = json!({"type":"broadcast","params":{"change":{"type":"patches",
            "baseRevision":1,"revision":2,"patches":[
                {"op":"add","path":["turns",0]},
                {"op":"replace","path":"/threadRuntimeStatus/activeFlags","value":["waitingOnUserInput"]},
                {"op":"replace","path":[],"value":{"id":"c","hostId":"local",
                    "threadRuntimeStatus":{"type":"idle"}}},
                {"value":{"type":"active","activeFlags":[]},"op":"replace",
                    "path":["threadRuntimeStatus"]},
                {"op":"replace","path":"/threadRuntimeStatus/type","value":"active"}]}}});
        assert_eq!(
            project(&serde_json::to_vec(&body).unwrap(), 5).unwrap(),
            expected
        );
    }

    #[test]
    fn live_projection_patch_value_before_its_path_stays_bounded_in_every_shape() {
        use super::super::protocol::{self, ThreadStatus};
        use crate::dto::LiveSessionState;
        let big = "x".repeat(2 * 1024 * 1024);
        let long_key = "k".repeat(KEY + 1);
        // >1MiB of distinct key names in one object.
        let many_keys = (0..70_000)
            .map(|i| format!("\"key-{i:012}\":0"))
            .collect::<Vec<_>>()
            .join(",");
        // Raw text: serde_json maps sort keys, which would put path first.
        let frame = |patch: &str| {
            format!(
                "{{\"type\":\"broadcast\",\"params\":{{\"change\":{{\"type\":\"patches\",\
                 \"baseRevision\":1,\"revision\":2,\"patches\":[{patch}]}}}}}}"
            )
            .into_bytes()
        };
        let patch_of = |bytes: &[u8]| {
            // One whole read and small reads must project the same way.
            let whole = project(bytes, bytes.len()).unwrap();
            assert_eq!(project(bytes, 4096).unwrap(), whole);
            whole["params"]["change"]["patches"][0].clone()
        };
        // Status after a running snapshot and then this one patch.
        let status_after = |bytes: &[u8]| {
            let mut thread = ThreadStatus::default();
            let snapshot = json!({"type":"broadcast","params":{"change":{"type":"snapshot",
                "revision":1,"conversationState":{"id":"c","hostId":"local",
                "threadRuntimeStatus":{"type":"active","activeFlags":[]}}}}});
            let first = protocol::parse(&serde_json::to_vec(&snapshot).unwrap()).unwrap();
            assert!(thread.apply("c", first.params.unwrap().change.unwrap()));
            let message = protocol::parse(bytes).unwrap();
            thread.apply("c", message.params.unwrap().change.unwrap());
            thread.status()
        };

        // Every value shape, on a path that does not touch status, in both
        // key orders: the frame is kept, nothing large is retained, and the
        // status is unchanged.
        let values = [
            format!("\"{big}\""),
            "12.5e-3".into(),
            "true".into(),
            "false".into(),
            "null".into(),
            format!("[\"{big}\"]"),
            format!("[[[\"{big}\"]],{{\"a\":\"{big}\"}}]"),
            format!("{{\"text\":\"{big}\",\"items\":[{{\"text\":\"{big}\"}}]}}"),
            format!("{{\"id\":\"{big}\",\"hostId\":{{\"a\":\"{big}\"}},\"type\":[\"{big}\"]}}"),
            format!(
                "{{\"threadRuntimeStatus\":{{\"type\":\"{big}\",\"activeFlags\":[\"{big}\"],\
                 \"other\":\"{big}\"}},\"activeFlags\":{{\"a\":\"{big}\"}}}}"
            ),
            format!("{{\"{long_key}\":\"{big}\",\"id\":\"turn\"}}"),
            format!("{{{many_keys}}}"),
        ];
        for value in &values {
            for patch in [
                format!("{{\"value\":{value},\"op\":\"add\",\"path\":[\"turns\",0]}}"),
                format!("{{\"op\":\"add\",\"path\":[\"turns\",0],\"value\":{value}}}"),
            ] {
                let bytes = frame(&patch);
                let projected = patch_of(&bytes);
                let size = serde_json::to_vec(&projected).unwrap().len();
                assert!(size < 400, "{size}: {}", &patch[..80]);
                assert_eq!(projected["path"], json!(["turns", 0]));
                assert_eq!(status_after(&bytes), LiveSessionState::Running);
            }
        }

        // Value-first still gives the same status as path-first wherever a
        // status path reads the value, including after replacements.
        for (path, value, expected) in [
            (
                "[]",
                format!(
                    "{{\"turns\":[\"{big}\"],\"id\":\"c\",\"hostId\":\"local\",\
                     \"threadRuntimeStatus\":{{\"type\":\"idle\",\"x\":\"{big}\"}}}}"
                ),
                LiveSessionState::Idle,
            ),
            (
                "[]",
                format!(
                    "{{\"id\":\"{big}\",\"hostId\":\"local\",\"threadRuntimeStatus\":{{\"type\":\"idle\"}}}}"
                ),
                LiveSessionState::Unknown,
            ),
            (
                "[\"threadRuntimeStatus\"]",
                format!(
                    "{{\"type\":\"active\",\"activeFlags\":[\"waitingOnApproval\"],\"turns\":\"{big}\"}}"
                ),
                LiveSessionState::WaitingApproval,
            ),
            (
                "[\"threadRuntimeStatus\"]",
                format!(
                    "{{\"type\":\"active\",\"activeFlags\":[\"waitingOnApproval\",\"{big}\"]}}"
                ),
                LiveSessionState::Unknown,
            ),
            (
                "\"/threadRuntimeStatus/type\"",
                "\"idle\"".into(),
                LiveSessionState::Idle,
            ),
            (
                "\"/threadRuntimeStatus/type\"",
                format!("\"{big}\""),
                LiveSessionState::Unknown,
            ),
            (
                "\"/threadRuntimeStatus/activeFlags\"",
                "[\"waitingOnUserInput\"]".into(),
                LiveSessionState::WaitingInput,
            ),
            (
                "\"/threadRuntimeStatus/activeFlags/-\"",
                "\"waitingOnApproval\"".into(),
                LiveSessionState::WaitingApproval,
            ),
            (
                "\"/threadRuntimeStatus/activeFlags/-\"",
                format!("\"{big}\""),
                LiveSessionState::Unknown,
            ),
        ] {
            // Appending a flag is an add; every other case replaces.
            let op = if path.ends_with("-\"") {
                "add"
            } else {
                "replace"
            };
            let first = frame(&format!(
                "{{\"value\":{value},\"op\":\"{op}\",\"path\":{path}}}"
            ));
            assert!(serde_json::to_vec(&patch_of(&first)).unwrap().len() < 400);
            assert_eq!(status_after(&first), expected, "{path}");
            // The path-first twin, where small enough to compare directly.
            if value.len() < 1024 {
                let twin = frame(&format!(
                    "{{\"op\":\"{op}\",\"path\":{path},\"value\":{value}}}"
                ));
                assert_eq!(status_after(&twin), expected, "{path}");
            }
        }

        // Arrays that could be flags stay intact; others become null.
        for (value, kept) in [
            (json!(["waitingOnApproval"]), json!(["waitingOnApproval"])),
            (
                json!(["waitingOnApproval", "waitingOnUserInput"]),
                json!(["waitingOnApproval", "waitingOnUserInput"]),
            ),
            (json!([]), json!([])),
            (json!(["a", "b", "c"]), Value::Null),
            (json!([1]), Value::Null),
            (json!([{"text":big}]), Value::Null),
            (json!(["waitingOnApproval", [big]]), Value::Null),
            (
                json!(["waitingOnApproval", big]),
                json!(["waitingOnApproval", null]),
            ),
        ] {
            let bytes = frame(&format!(
                "{{\"value\":{value},\"op\":\"replace\",\"path\":\"/threadRuntimeStatus/activeFlags\"}}"
            ));
            assert_eq!(patch_of(&bytes)["value"], kept);
        }
    }

    #[test]
    fn live_projection_rejects_malformed_json_and_limits() {
        for bad in [
            &b""[..],
            b"[]",
            b"{",
            b"{}x",
            b"{\"a\":}",
            b"{\"a\":1,}",
            b"{\"a\" 1}",
            b"{\"a\":01}",
            b"{\"a\":1.}",
            b"{\"a\":-}",
            b"{\"a\":1e}",
            b"{\"a\":tru}",
            b"{\"a\":\"\\x\"}",
            b"{\"a\":\"\\uD800\"}",
            b"{\"a\":\"\\uDC00\"}",
            b"{\"a\":\"\xc0\x80\"}",
            b"{\"a\":\"\xed\xa0\x80\"}",
            b"{\"a\":\"\x01\"}",
            b"{\"type\":1,\"type\":2}",
            b"{\"a\":[1 2]}",
        ] {
            assert_eq!(
                project(bad, 1),
                Err(Invalid),
                "{}",
                String::from_utf8_lossy(bad)
            );
        }
        let deep = format!("{{\"skip\":{}0{}}}", "[".repeat(127), "]".repeat(127));
        assert!(project(deep.as_bytes(), 9).is_ok());
        let deep = format!("{{\"skip\":{}0{}}}", "[".repeat(128), "]".repeat(128));
        assert_eq!(project(deep.as_bytes(), 9), Err(Invalid));
        let long_key = format!("{{\"{}\":1}}", "k".repeat(KEY + 1));
        assert_eq!(project(long_key.as_bytes(), 3), Err(Invalid));
        // A kept field cannot smuggle a transcript past the projection bound.
        let kept = format!("{{\"status\":\"{}\"}}", "x".repeat(MAX_PROJECTION));
        assert_eq!(project(kept.as_bytes(), 4096), Err(Invalid));
        let number = format!("{{\"skip\":{}}}", "1".repeat(NUMBER + 1));
        assert_eq!(project(number.as_bytes(), 3), Err(Invalid));
    }
}
