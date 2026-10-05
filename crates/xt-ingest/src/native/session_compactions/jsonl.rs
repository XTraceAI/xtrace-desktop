//! Compaction-only JSONL projection. Bodies are validated and discarded as
//! they arrive; neither a line nor a skipped string is ever collected.
//!
//! serde's IgnoredAny streams strings, but its ignored nesting stack is
//! unbounded and from_reader still collects keys/retained strings before a
//! DeserializeSeed can check their size. This scanner bounds those first and
//! hands only the small metadata projection to serde_json.
use super::{MAX_DEPTH, Reason, Work};
use serde_json::Value;
use std::{collections::BTreeSet, io::Read};

const CHUNK: usize = 64 * 1024;
const METADATA: usize = 64 * 1024;
const KEY: usize = 256;

#[derive(Clone, Copy)]
enum Projection {
    Claude,
    Codex,
    Payload,
    Header,
    HeaderPayload,
}
impl Projection {
    fn keeps(self, key: &str) -> bool {
        let ownership = matches!(
            key,
            "originalSessionId"
                | "sourceSessionId"
                | "forkedFromSessionId"
                | "parentSessionId"
                | "parentComposerId"
                | "isFork"
        );
        ownership && !matches!(self, Self::Codex | Self::Header)
            || match self {
                Self::Claude => matches!(
                    key,
                    "type" | "subtype" | "uuid" | "sessionId" | "isSidechain"
                ),
                Self::Codex | Self::Header => matches!(key, "type" | "ordinal" | "payload"),
                Self::Payload => matches!(key, "compaction_response_id" | "window_id"),
                Self::HeaderPayload => matches!(
                    key,
                    "id" | "session_id"
                        | "forked_from_id"
                        | "source"
                        | "history_mode"
                        | "history_base"
                        | "subagent_history_start_ordinal"
                        | "type"
                        | "call_id"
                        | "name"
                ),
            }
    }
}

/// Bytes kept of a compaction's recorded time (RFC 3339 is at most ~35).
pub(super) const SIDE_TIME: usize = 40;
/// Bytes kept of its trigger, and of each compactMetadata key compared to
/// "trigger": enough for "manual"/"trigger" and nothing more.
pub(super) const SIDE_TRIGGER: usize = 8;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SlotState {
    Empty,
    Ok,
    Bad,
}
/// A fixed-size copy of one short ASCII string value seen while a row is
/// scanned. It never touches the projection, its limits or its errors: an
/// escape, a non-ASCII byte, overflow, a non-string or a repeat is `Bad`.
#[derive(Clone, Copy, Debug)]
pub(super) struct Slot<const N: usize> {
    bytes: [u8; N],
    len: usize,
    state: SlotState,
}
impl<const N: usize> Slot<N> {
    const EMPTY: Self = Self {
        bytes: [0; N],
        len: 0,
        state: SlotState::Empty,
    };
    fn begin(&mut self) {
        self.state = if self.state == SlotState::Empty {
            SlotState::Ok
        } else {
            SlotState::Bad
        };
        self.len = 0;
    }
    fn push(&mut self, bytes: &[u8]) {
        if self.state != SlotState::Ok {
            return;
        }
        if self.len + bytes.len() > N || !bytes.iter().all(|b| (32..127).contains(b)) {
            self.state = SlotState::Bad;
        } else {
            self.bytes[self.len..self.len + bytes.len()].copy_from_slice(bytes);
            self.len += bytes.len();
        }
    }
    fn bad(&mut self) {
        self.state = SlotState::Bad;
    }
    /// The whole value, when it was one short plain string seen once.
    pub(super) fn get(&self) -> Option<&str> {
        (self.state == SlotState::Ok)
            .then(|| std::str::from_utf8(&self.bytes[..self.len]).ok())
            .flatten()
    }
}
/// Display-only facts of the row just read, kept beside its projection.
#[derive(Clone, Copy, Debug)]
pub(super) struct Side {
    pub(super) timestamp: Slot<SIDE_TIME>,
    pub(super) trigger: Slot<SIDE_TRIGGER>,
}
impl Side {
    const EMPTY: Self = Self {
        timestamp: Slot::EMPTY,
        trigger: Slot::EMPTY,
    };
}
/// What, besides the projection, one value is copied into.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    None,
    Timestamp,
    /// The value of a Claude row's compactMetadata: its keys are compared
    /// with "trigger", one level down only.
    Metadata,
    Trigger,
    /// A compactMetadata key, copied to compare it with "trigger".
    Key,
}
fn record(side: &mut Side, key: &mut Slot<SIDE_TRIGGER>, mode: Mode, bytes: &[u8]) {
    match mode {
        Mode::Timestamp => side.timestamp.push(bytes),
        Mode::Trigger => side.trigger.push(bytes),
        Mode::Key => key.push(bytes),
        Mode::None | Mode::Metadata => {}
    }
}
fn spoil(side: &mut Side, key: &mut Slot<SIDE_TRIGGER>, mode: Mode) {
    match mode {
        Mode::Timestamp => side.timestamp.bad(),
        Mode::Trigger => side.trigger.bad(),
        Mode::Key => key.bad(),
        Mode::None | Mode::Metadata => {}
    }
}

pub(super) struct Rows<'a, 'b, R> {
    source: R,
    work: &'a mut Work<'b>,
    buffer: Box<[u8; CHUNK]>,
    at: usize,
    end: usize,
    remaining: u64,
    offset: u64,
    output: Vec<u8>,
    metadata: usize,
    side: Side,
    key: Slot<SIDE_TRIGGER>,
}
impl<'a, 'b, R: Read> Rows<'a, 'b, R> {
    pub(super) fn new(source: R, length: u64, work: &'a mut Work<'b>) -> Self {
        Self {
            source,
            work,
            buffer: Box::new([0; CHUNK]),
            at: 0,
            end: 0,
            remaining: length,
            offset: 0,
            output: Vec::new(),
            metadata: 0,
            side: Side::EMPTY,
            key: Slot::EMPTY,
        }
    }
    fn peek(&mut self) -> Result<Option<u8>, Reason> {
        if self.at == self.end {
            self.work.check()?;
            if self.remaining == 0 {
                return Ok(None);
            }
            let wanted = self.remaining.min(CHUNK as u64) as usize;
            // Charge before reading, including failed/partial histories.
            self.work.charge(wanted as u64)?;
            self.end = self
                .source
                .read(&mut self.buffer[..wanted])
                .map_err(|_| Reason::Unreadable)?;
            if self.end == 0 {
                return Err(Reason::Replaced);
            }
            self.remaining -= self.end as u64;
            self.at = 0;
        }
        Ok(Some(self.buffer[self.at]))
    }
    fn byte(&mut self) -> Result<u8, Reason> {
        let byte = self.peek()?.ok_or(Reason::Incomplete)?;
        self.advance(1);
        Ok(byte)
    }
    fn advance(&mut self, n: usize) {
        self.at += n;
        self.offset += n as u64;
    }
    fn expect(&mut self, byte: u8) -> Result<(), Reason> {
        if self.byte()? == byte {
            Ok(())
        } else {
            Err(Reason::Incomplete)
        }
    }
    fn space(&mut self) -> Result<(), Reason> {
        while matches!(self.peek()?, Some(b' ' | b'\t' | b'\r')) {
            self.advance(1);
        }
        Ok(())
    }
    fn emit(&mut self, bytes: &[u8], keep: bool) -> Result<(), Reason> {
        if keep {
            if self.output.len().saturating_add(bytes.len()) > METADATA {
                return Err(Reason::Limit);
            }
            self.output.extend_from_slice(bytes);
        }
        Ok(())
    }
    fn hex(&mut self, keep: bool) -> Result<u16, Reason> {
        let mut value = 0;
        for _ in 0..4 {
            let byte = self.byte()?;
            self.emit(&[byte], keep)?;
            let digit = (byte as char).to_digit(16).ok_or(Reason::Incomplete)?;
            value = value * 16 + digit as u16;
        }
        Ok(value)
    }
    fn string(&mut self, keep: bool, key: bool, mode: Mode) -> Result<(), Reason> {
        self.expect(b'"')?;
        match mode {
            Mode::Timestamp => self.side.timestamp.begin(),
            Mode::Trigger => self.side.trigger.begin(),
            Mode::Key => {
                self.key = Slot::EMPTY;
                self.key.begin();
            }
            Mode::None | Mode::Metadata => {}
        }
        self.emit(b"\"", keep)?;
        let start = self.offset;
        loop {
            let byte = self.peek()?.ok_or(Reason::Incomplete)?;
            if key && self.offset - start > KEY as u64 {
                return Err(Reason::Limit);
            }
            match byte {
                b'"' => {
                    self.advance(1);
                    return self.emit(b"\"", keep);
                }
                b'\\' => {
                    spoil(&mut self.side, &mut self.key, mode);
                    self.advance(1);
                    let escaped = self.byte()?;
                    self.emit(&[b'\\', escaped], keep)?;
                    match escaped {
                        b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => {}
                        b'u' => {
                            let code = self.hex(keep)?;
                            if (0xd800..=0xdbff).contains(&code) {
                                self.expect(b'\\')?;
                                self.expect(b'u')?;
                                self.emit(b"\\u", keep)?;
                                if !(0xdc00..=0xdfff).contains(&self.hex(keep)?) {
                                    return Err(Reason::Incomplete);
                                }
                            } else if (0xdc00..=0xdfff).contains(&code) {
                                return Err(Reason::Incomplete);
                            }
                        }
                        _ => return Err(Reason::Incomplete),
                    }
                }
                0..=31 => return Err(Reason::Incomplete),
                128..=255 => {
                    spoil(&mut self.side, &mut self.key, mode);
                    // Validate one UTF-8 codepoint, also across read boundaries.
                    let len = match byte {
                        0xc2..=0xdf => 2,
                        0xe0..=0xef => 3,
                        0xf0..=0xf4 => 4,
                        _ => return Err(Reason::Incomplete),
                    };
                    let mut code = [0; 4];
                    for item in &mut code[..len] {
                        *item = self.byte()?;
                    }
                    std::str::from_utf8(&code[..len]).map_err(|_| Reason::Incomplete)?;
                    self.emit(&code[..len], keep)?;
                }
                _ => {
                    // The common, potentially enormous ASCII run is handled
                    // a chunk at a time, with no per-byte allocation or I/O.
                    let run = &self.buffer[self.at..self.end];
                    let n = run
                        .iter()
                        .position(|b| *b < 32 || *b >= 128 || matches!(*b, b'"' | b'\\'))
                        .unwrap_or(run.len());
                    if key && self.offset - start + n as u64 > KEY as u64 {
                        return Err(Reason::Limit);
                    }
                    if keep {
                        if self.output.len().saturating_add(n) > METADATA {
                            return Err(Reason::Limit);
                        }
                        self.output.extend_from_slice(&run[..n]);
                    }
                    record(
                        &mut self.side,
                        &mut self.key,
                        mode,
                        &self.buffer[self.at..self.at + n],
                    );
                    self.advance(n);
                }
            }
        }
    }
    fn value(
        &mut self,
        keep: bool,
        projection: Option<Projection>,
        depth: usize,
        mode: Mode,
    ) -> Result<(), Reason> {
        if depth > MAX_DEPTH {
            return Err(Reason::Limit);
        }
        self.space()?;
        let first = self.peek()?.ok_or(Reason::Incomplete)?;
        if first != b'"' {
            spoil(&mut self.side, &mut self.key, mode);
        }
        match first {
            b'"' => self.string(keep, false, mode),
            b'{' => {
                self.advance(1);
                self.emit(b"{", keep)?;
                let mut seen = BTreeSet::new();
                let mut first = true;
                self.space()?;
                if self.peek()? != Some(b'}') {
                    loop {
                        let key_start = self.output.len();
                        let inspect = keep || projection.is_some();
                        let key_mode = if mode == Mode::Metadata {
                            Mode::Key
                        } else {
                            Mode::None
                        };
                        self.string(inspect, inspect, key_mode)?;
                        let key = if inspect {
                            let bytes = &self.output[key_start..];
                            self.metadata = self.metadata.saturating_add(bytes.len());
                            if self.metadata > METADATA {
                                return Err(Reason::Limit);
                            }
                            let key: String =
                                serde_json::from_slice(bytes).map_err(|_| Reason::Incomplete)?;
                            self.output.truncate(key_start);
                            if !seen.insert(key.clone()) {
                                return Err(Reason::Incomplete);
                            }
                            Some(key)
                        } else {
                            None
                        };
                        self.space()?;
                        self.expect(b':')?;
                        let selected = keep
                            && projection.is_none_or(|p| p.keeps(key.as_deref().unwrap_or("")));
                        if selected {
                            if !first {
                                self.emit(b",", true)?;
                            }
                            let key = serde_json::to_vec(key.as_ref().ok_or(Reason::Incomplete)?)
                                .map_err(|_| Reason::Incomplete)?;
                            self.emit(&key, true)?;
                            self.emit(b":", true)?;
                            first = false;
                        }
                        let child = match (projection, key.as_deref()) {
                            (Some(Projection::Codex), Some("payload")) => Some(Projection::Payload),
                            (Some(Projection::Header), Some("payload")) => {
                                Some(Projection::HeaderPayload)
                            }
                            _ => None,
                        };
                        // Display-only copies; the projection is unchanged.
                        let side = match (mode, projection, key.as_deref()) {
                            (Mode::Metadata, _, _) if self.key.get() == Some("trigger") => {
                                Mode::Trigger
                            }
                            (
                                Mode::None,
                                Some(Projection::Claude | Projection::Codex),
                                Some("timestamp"),
                            ) => Mode::Timestamp,
                            (Mode::None, Some(Projection::Claude), Some("compactMetadata")) => {
                                Mode::Metadata
                            }
                            _ => Mode::None,
                        };
                        self.value(selected, child, depth + 1, side)?;
                        self.space()?;
                        match self.byte()? {
                            b'}' => break,
                            b',' => self.space()?,
                            _ => return Err(Reason::Incomplete),
                        }
                    }
                } else {
                    self.advance(1);
                }
                self.emit(b"}", keep)
            }
            b'[' => {
                self.advance(1);
                self.emit(b"[", keep)?;
                self.space()?;
                if self.peek()? != Some(b']') {
                    loop {
                        self.value(keep, None, depth + 1, Mode::None)?;
                        self.space()?;
                        match self.byte()? {
                            b']' => break,
                            b',' => self.emit(b",", keep)?,
                            _ => return Err(Reason::Incomplete),
                        }
                    }
                } else {
                    self.advance(1);
                }
                self.emit(b"]", keep)
            }
            b'n' | b't' | b'f' => {
                let literal: &[u8] = match self.peek()? {
                    Some(b'n') => b"null",
                    Some(b't') => b"true",
                    _ => b"false",
                };
                for byte in literal {
                    self.expect(*byte)?;
                }
                self.emit(literal, keep)
            }
            b'-' | b'0'..=b'9' => {
                let mut number = Vec::new();
                while let Some(byte @ (b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')) =
                    self.peek()?
                {
                    if number.len() == 128 {
                        return Err(Reason::Limit);
                    }
                    number.push(byte);
                    self.advance(1);
                }
                serde_json::from_slice::<serde_json::Number>(&number)
                    .map_err(|_| Reason::Incomplete)?;
                self.emit(&number, keep)
            }
            _ => Err(Reason::Incomplete),
        }
    }
    fn projected(&mut self, projection: Projection) -> Result<Option<(Value, u64)>, Reason> {
        self.work.check()?;
        if self.peek()?.is_none() {
            return Ok(None);
        }
        self.output.clear();
        self.side = Side::EMPTY;
        self.metadata = 0;
        self.space()?;
        if self.peek()? != Some(b'{') {
            return Err(Reason::Incomplete);
        }
        self.value(true, Some(projection), 0, Mode::None)?;
        self.space()?;
        self.expect(b'\n')?;
        let row = serde_json::from_slice(&self.output).map_err(|_| Reason::Incomplete)?;
        Ok(Some((row, self.offset)))
    }
    pub(super) fn next(&mut self, codex: bool) -> Result<Option<(Value, u64)>, Reason> {
        self.projected(if codex {
            Projection::Codex
        } else {
            Projection::Claude
        })
    }
    /// Display-only facts of the row `next` last returned.
    pub(super) fn side(&self) -> Side {
        self.side
    }
    pub(super) fn header(&mut self) -> Result<Value, Reason> {
        self.projected(Projection::Header)?
            .map(|(row, _)| row)
            .ok_or(Reason::Incomplete)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::{claude_launch::group, readers_cli::CancelToken};
    use std::{
        io::{self, Cursor},
        time::{Duration, Instant},
    };

    fn work(cancel: &CancelToken) -> Work<'_> {
        Work {
            deadline: Instant::now() + Duration::from_secs(15),
            cancel,
            bytes: 0,
            decoded: 0,
            nodes: 0,
        }
    }
    const ID: &str = "00000000-0000-4000-8000-000000000001";

    #[test]
    fn header_projection_matches_existing_output_and_structure_checks() {
        // OutputHead and PartHead accept every JSON type. The header does
        // not consult their contents, so skipping output preserves that
        // behavior; keeping it would bring large message bodies back.
        let values = [
            Value::Null,
            Value::Bool(false),
            serde_json::json!(1.5),
            serde_json::json!({"anything":[false]}),
            serde_json::json!([{"type":7,"text":{}}]),
            serde_json::json!("body"),
        ];
        let cancel = CancelToken::new();
        for output in values {
            let value = serde_json::json!({"type":"session_meta","ordinal":0,
                "payload":{"id":ID,"history_mode":"paginated","output":output}});
            let original = serde_json::to_vec(&value).unwrap();
            let before = group::header(&original, ID).unwrap();
            let mut line = original;
            line.push(b'\n');
            let after = Rows::new(Cursor::new(&line), line.len() as u64, &mut work(&cancel))
                .header()
                .unwrap();
            let after = group::header(&serde_json::to_vec(&after).unwrap(), ID).unwrap();
            assert_eq!(before, after);
        }
        // PayloadHead's string fields, unlike output, reject these types.
        for field in ["type", "call_id", "name"] {
            let mut value = serde_json::json!({"type":"session_meta","payload":{"id":ID}});
            value["payload"][field] = serde_json::json!(false);
            let original = serde_json::to_vec(&value).unwrap();
            assert!(group::header(&original, ID).is_err());
            let mut line = original;
            line.push(b'\n');
            let after = Rows::new(Cursor::new(&line), line.len() as u64, &mut work(&cancel))
                .header()
                .unwrap();
            assert!(group::header(&serde_json::to_vec(&after).unwrap(), ID).is_err());
        }
    }

    #[test]
    fn huge_strings_use_the_same_fixed_buffer_and_metadata_capacity() {
        let cancel = CancelToken::new();
        let mut capacities = Vec::new();
        for size in [1024 * 1024, 32 * 1024 * 1024] {
            let prefix = b"{\"payload\":{\"message\":\"";
            let suffix =
                b"\",\"compaction_response_id\":\"own\"},\"type\":\"compacted\",\"ordinal\":1}\n";
            let input = prefix
                .as_slice()
                .chain(io::repeat(b'x').take(size))
                .chain(suffix.as_slice());
            let length = prefix.len() as u64 + size + suffix.len() as u64;
            let mut work = work(&cancel);
            let mut rows = Rows::new(input, length, &mut work);
            let (row, end) = rows.next(true).unwrap().unwrap();
            assert_eq!(end, length);
            assert_eq!(row["payload"]["compaction_response_id"], "own");
            assert_eq!(rows.buffer.len(), CHUNK);
            assert!(rows.output.capacity() <= METADATA);
            capacities.push(rows.output.capacity());
        }
        assert_eq!(capacities[0], capacities[1]);
    }

    #[test]
    fn huge_header_output_is_skipped_and_oversized_required_metadata_is_unknown() {
        let cancel = CancelToken::new();
        let prefix = b"{\"payload\":{\"output\":\"";
        let suffix = format!("\",\"id\":\"{ID}\"}},\"type\":\"session_meta\"}}\n");
        let size = 2 * 1024 * 1024;
        let input = prefix
            .as_slice()
            .chain(io::repeat(b'x').take(size))
            .chain(suffix.as_bytes());
        let length = prefix.len() as u64 + size + suffix.len() as u64;
        let header = Rows::new(input, length, &mut work(&cancel))
            .header()
            .unwrap();
        assert!(group::header(&serde_json::to_vec(&header).unwrap(), ID).is_ok());
        for prefix in [
            b"{\"payload\":{\"compaction_response_id\":\"".as_slice(),
            b"{\"",
        ] {
            let input = prefix.chain(io::repeat(b'x').take(1024 * 1024));
            assert_eq!(
                Rows::new(input, 1024 * 1024, &mut work(&cancel))
                    .next(true)
                    .unwrap_err(),
                Reason::Limit
            );
        }
    }

    struct CancelOnSecondRead<R> {
        input: R,
        cancel: CancelToken,
        reads: usize,
    }
    impl<R: Read> Read for CancelOnSecondRead<R> {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            if self.reads == 2 {
                self.cancel.cancel();
            }
            self.input.read(bytes)
        }
    }
    #[test]
    fn cancellation_deadline_and_shared_budget_interrupt_long_skips() {
        let prefix = b"{\"payload\":{\"message\":\"";
        let cancel = CancelToken::new();
        let input = CancelOnSecondRead {
            input: prefix.as_slice().chain(io::repeat(b'x')),
            cancel: cancel.clone(),
            reads: 0,
        };
        let mut work = work(&cancel);
        assert_eq!(
            Rows::new(input, 1024 * 1024, &mut work)
                .next(true)
                .unwrap_err(),
            Reason::Cancelled
        );
        assert_eq!(work.bytes, 2 * CHUNK as u64);

        let cancel = CancelToken::new();
        let mut work = self::work(&cancel);
        work.deadline = Instant::now();
        assert_eq!(
            Rows::new(prefix.as_slice(), prefix.len() as u64, &mut work)
                .next(true)
                .unwrap_err(),
            Reason::Limit
        );
        assert_eq!(work.bytes, 0);

        work.deadline = Instant::now() + Duration::from_secs(15);
        let malformed = b"{broken";
        assert_eq!(
            Rows::new(malformed.as_slice(), malformed.len() as u64, &mut work)
                .next(true)
                .unwrap_err(),
            Reason::Incomplete
        );
        assert_eq!(work.bytes, malformed.len() as u64); // Failed attempts are charged.
        work.bytes = super::super::MAX_BATCH - CHUNK as u64;
        let input = prefix.as_slice().chain(io::repeat(b'x'));
        assert_eq!(
            Rows::new(input, 1024 * 1024, &mut work)
                .next(true)
                .unwrap_err(),
            Reason::Limit
        );
        assert!(work.bytes > super::super::MAX_BATCH);
    }

    #[test]
    fn unicode_and_escapes_cross_read_boundaries_without_false_markers() {
        let cancel = CancelToken::new();
        for ending in ["é", "😀", r#"\uD83D\uDE00"#, r#"\"type\":\"compacted\""#] {
            let prefix = "{\"type\":\"response_item\",\"payload\":{\"message\":\"";
            let mut bytes = prefix.as_bytes().to_vec();
            bytes.resize(CHUNK - 1, b'x');
            bytes.extend_from_slice(ending.as_bytes());
            bytes.extend_from_slice(b"\"},\"ordinal\":1}\r\n");
            let (row, offset) =
                Rows::new(Cursor::new(&bytes), bytes.len() as u64, &mut work(&cancel))
                    .next(true)
                    .unwrap()
                    .unwrap();
            assert_eq!(row["type"], "response_item");
            assert_eq!(row["ordinal"], 1);
            assert_eq!(offset, bytes.len() as u64);
        }
    }

    #[test]
    fn source_replacement_and_rewrite_are_detected_after_streaming() {
        use super::super::{open_regular, regular, verify_jsonl};
        use std::fs;
        let directory = tempfile::Builder::new().tempdir_in("/private/tmp").unwrap();
        let path = directory.path().join("history.jsonl");
        let bytes = b"{\"type\":\"response_item\",\"payload\":{},\"ordinal\":1}\n";
        fs::write(&path, bytes).unwrap();
        let (mut file, identity) = open_regular(&path, &regular(&path).unwrap()).unwrap();
        let cancel = CancelToken::new();
        assert!(
            Rows::new(&mut file, identity.len, &mut work(&cancel))
                .next(true)
                .unwrap()
                .is_some()
        );
        fs::write(&path, bytes).unwrap(); // Same bytes still move the change time.
        assert_eq!(
            verify_jsonl(&file, &path, &identity).unwrap_err(),
            Reason::Replaced
        );
        let (file, identity) = open_regular(&path, &regular(&path).unwrap()).unwrap();
        let replacement = directory.path().join("new.jsonl");
        fs::write(&replacement, bytes).unwrap();
        fs::rename(replacement, &path).unwrap();
        assert_eq!(
            verify_jsonl(&file, &path, &identity).unwrap_err(),
            Reason::Replaced
        );
    }

    /// The projection as base 80920b1 defined it, computed independently
    /// from a full parse: the side record must leave it untouched.
    fn base_projection(line: &str, codex: bool) -> Value {
        const OWNERSHIP: [&str; 6] = [
            "originalSessionId",
            "sourceSessionId",
            "forkedFromSessionId",
            "parentSessionId",
            "parentComposerId",
            "isFork",
        ];
        let Value::Object(row) = serde_json::from_str(line).unwrap() else {
            panic!("rows are objects")
        };
        let kept: &[&str] = if codex {
            &["type", "ordinal", "payload"]
        } else {
            &["type", "subtype", "uuid", "sessionId", "isSidechain"]
        };
        Value::Object(
            row.into_iter()
                .filter(|(k, _)| {
                    kept.contains(&k.as_str()) || !codex && OWNERSHIP.contains(&k.as_str())
                })
                .map(|(k, v)| match (codex, k.as_str(), v) {
                    (true, "payload", Value::Object(payload)) => (
                        k,
                        Value::Object(
                            payload
                                .into_iter()
                                .filter(|(k, _)| {
                                    matches!(k.as_str(), "compaction_response_id" | "window_id")
                                        || OWNERSHIP.contains(&k.as_str())
                                })
                                .collect(),
                        ),
                    ),
                    (_, _, v) => (k, v),
                })
                .collect(),
        )
    }

    #[test]
    fn side_record_leaves_the_projection_offsets_and_errors_as_base() {
        let cancel = CancelToken::new();
        let wide = "u".repeat(65_440);
        let big = "x".repeat(80 * 1024);
        let keys: String = (0..20_000).map(|n| format!("\"k{n}\":0,")).collect();
        let rows: Vec<(bool, String, Option<&str>, Option<&str>)> = vec![
            (false, format!(r#"{{"timestamp":"2026-09-07T12:00:00Z","type":"system","subtype":"compact_boundary","uuid":"{ID}","sessionId":"{ID}","isSidechain":false,"compactMetadata":{{"trigger":"auto","preTokens":1}}}}"#), Some("2026-09-07T12:00:00Z"), Some("auto")),
            (false, format!(r#"{{"timestamp":"2026-09-07T12:00:00Z","type":"progress","sessionId":"{ID}","uuid":"{wide}"}}"#), Some("2026-09-07T12:00:00Z"), None),
            (false, format!(r#"{{"compactMetadata":"{big}","timestamp":12,"type":"system"}}"#), None, None),
            (false, format!(r#"{{"compactMetadata":[{{"trigger":"auto"}}],"timestamp":"{big}"}}"#), None, None),
            (false, format!(r#"{{"compactMetadata":{{"trigger":"{big}"}},"timestamp":"a\u00e9"}}"#), None, None),
            (false, format!(r#"{{"compactMetadata":{{{keys}"nested":{{"trigger":"auto"}},"trigger":"manual"}},"originalSessionId":"{ID}"}}"#), None, Some("manual")),
            (false, r#"{"compactMetadata":{"trigger":"auto","trigger":"manual"},"timestamp":"t\"q"}"#.to_owned(), None, None),
            (false, r#"{"compactMetadata":{"trigger":["auto"]},"timestamp":["t"]}"#.to_owned(), None, None),
            (false, r#"{"compactMetadata":{"trigger":"autoé"},"timestamp":"2026-09-07T12:00:00é"}"#.to_owned(), None, None),
            (false, r#"{"compactMetadata":{"trig\u0067er":"auto"}}"#.to_owned(), None, None),
            (true, format!(r#"{{"timestamp":"2026-09-07T12:00:00.5Z","type":"compacted","ordinal":2,"payload":{{"message":"{big}","window_id":"w","timestamp":"no"}}}}"#), Some("2026-09-07T12:00:00.5Z"), None),
            (true, r#"{"type":"compacted","compactMetadata":{"trigger":"auto"},"timestamp":7,"payload":[1]}"#.to_owned(), None, None),
        ];
        for (codex, line, time, trigger) in rows {
            let mut bytes = line.clone().into_bytes();
            bytes.push(b'\n');
            let mut work = work(&cancel);
            let mut scan = Rows::new(Cursor::new(&bytes), bytes.len() as u64, &mut work);
            let (value, offset) = scan.next(codex).unwrap().unwrap();
            assert_eq!(value, base_projection(&line, codex), "{}", &line[..60]);
            assert_eq!(offset, bytes.len() as u64);
            let side = scan.side();
            assert_eq!(side.timestamp.get(), time, "{}", &line[..60]);
            assert_eq!(side.trigger.get(), trigger, "{}", &line[..60]);
            // The next row starts with an empty side record.
            assert!(scan.next(codex).unwrap().is_none());
        }
        // Errors are those of base, whatever the side record saw first.
        for (line, error) in [
            (r#"{"timestamp":"a","timestamp":"b"}"#, Reason::Incomplete),
            (
                r#"{"timestamp":"2026","compactMetadata":{"trigger":"auto",}}"#,
                Reason::Incomplete,
            ),
            ("{\"timestamp\":\"20\u{1}26\"}", Reason::Incomplete),
            (
                r#"{"compactMetadata":{"trigger":"auto"},"type":"#,
                Reason::Incomplete,
            ),
        ] {
            let mut bytes = line.as_bytes().to_vec();
            bytes.push(b'\n');
            assert_eq!(
                Rows::new(Cursor::new(&bytes), bytes.len() as u64, &mut work(&cancel))
                    .next(false)
                    .unwrap_err(),
                error,
                "{line}"
            );
        }
        // A wider field that base already refused is still refused.
        let line = format!(r#"{{"type":"progress","uuid":"{}"}}"#, "u".repeat(65_520));
        let mut bytes = line.into_bytes();
        bytes.push(b'\n');
        assert_eq!(
            Rows::new(Cursor::new(&bytes), bytes.len() as u64, &mut work(&cancel))
                .next(false)
                .unwrap_err(),
            Reason::Limit
        );
    }
}
