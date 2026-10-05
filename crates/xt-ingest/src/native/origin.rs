//! Opt-in Codex origin evidence riding on the shared reader stream.
//!
//! In origin-evidence mode the pinned producer marks a session header with
//! `origin_evidence: {contract, version}` and adds one closed `origin_evidence`
//! object to each record line it claims Codex itself injected (version 1: the
//! instructions of a selected skill). This module only judges that evidence
//! against the line's own parsed record and session; it never changes, drops or
//! delays the record. A claim that is malformed, mixed, misbound or repeated is
//! refused for the proof alone, with a fixed metadata-only code. Nothing here
//! reads, keeps or echoes text, a text digest, a path or a title.

use serde::{
    Deserialize, Deserializer,
    de::{self, MapAccess, SeqAccess, Visitor},
};
use serde_json::{Map, Value};
use std::{
    collections::{HashMap, HashSet},
    fmt,
};
use xt_store::{
    CanonicalRecord,
    injected::{
        InjectedContextKind, InjectedContextProof, ORIGIN_EVIDENCE_VERSION, OriginContract,
        SegmentHistory,
    },
    model::RecordType,
};

/// What a session header states about origin evidence. Any present value is
/// judged on arrival and not retained; `null` is present, not absent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OriginMarker {
    #[default]
    Absent,
    /// Exactly `{"contract":"memhub.codex.origin_evidence","version":1}`.
    V1,
    Invalid,
}

impl<'de> Deserialize<'de> for OriginMarker {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Unrepeated(value) = Unrepeated::deserialize(deserializer)?;
        let v1 = value
            .as_ref()
            .and_then(Value::as_object)
            .is_some_and(|object| {
                object.len() == 2
                    && object.get("contract").and_then(Value::as_str)
                        == Some(OriginContract::CodexOriginEvidence.as_str())
                    && version_one(object.get("version"))
            });
        Ok(if v1 { Self::V1 } else { Self::Invalid })
    }
}

/// What the evidence says of one session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionOrigin {
    /// The reader examined every record of this session.
    Marked,
    /// No marker: the reader withheld evidence for this session (or ran
    /// without it). Its records are unclassified, not the person's own.
    Withheld,
    /// A marker that is not exactly version 1. No record of it is claimed.
    Invalid,
}

/// What the evidence says of one record. Only `Claimed` carries a proof.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordOrigin {
    /// A validated claim bound to this very record and session.
    Claimed(Box<InjectedContextProof>),
    /// A marked session's record without a claim: examined and declined.
    Declined,
    /// An unmarked session's record without a claim: evidence absent.
    Withheld,
    /// The evidence about this record is refused. The record itself stands.
    Invalid(OriginInvalid),
}

/// Fixed refusal codes. None echoes a value from the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OriginInvalid {
    /// The session's marker is not the version 1 marker.
    SessionMarker,
    /// A claim in a session whose header carries no marker.
    Unmarked,
    /// Not one closed object: missing, unknown or repeated keys.
    Shape,
    Contract,
    Version,
    Kind,
    /// A session, item, turn or record identifier not in its native shape.
    Identifier,
    Segment,
    Row,
    /// The claim names another session than its header.
    SessionMismatch,
    /// The claim names another record than its line.
    RecordMismatch,
    /// The claimed record is not a user record.
    NotUserRecord,
    /// The record, item or row was already claimed or seen in this session.
    /// On an unclaimed record it disputes the earlier claim of its UUID.
    Duplicate,
}

impl OriginInvalid {
    pub fn code(self) -> &'static str {
        match self {
            Self::SessionMarker => "origin_session_marker_invalid",
            Self::Unmarked => "origin_claim_unmarked_session",
            Self::Shape => "origin_claim_shape",
            Self::Contract => "origin_claim_contract",
            Self::Version => "origin_claim_version",
            Self::Kind => "origin_claim_kind",
            Self::Identifier => "origin_claim_identifier",
            Self::Segment => "origin_claim_segment",
            Self::Row => "origin_claim_row",
            Self::SessionMismatch => "origin_claim_session_mismatch",
            Self::RecordMismatch => "origin_claim_record_mismatch",
            Self::NotUserRecord => "origin_claim_not_user_record",
            Self::Duplicate => "origin_claim_duplicate",
        }
    }
}

/// The evidence sidecar of one stream event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Session(SessionOrigin),
    Record(RecordOrigin),
}

/// Per-session bookkeeping. Only identities are held, and only until the next
/// header: every record UUID of a marked session, the claimed identities, and
/// the identities any other claim line of the session named.
pub(super) struct OriginSession {
    state: SessionOrigin,
    native_session_id: String,
    seen: HashSet<String>,
    claimed: HashSet<String>,
    /// Each claimed item and row, to the UUID of the record that claimed it.
    items: HashMap<String, String>,
    rows: HashMap<Row, String>,
    /// The claimed UUIDs at each row index, whatever the rollout.
    indices: HashMap<u32, Vec<String>>,
    /// Identities every claim line of the session named, accepted or not,
    /// however malformed.
    named: Named,
    /// Claimed UUIDs a line disputed since the consumer last took them.
    disputes: Vec<String>,
}

type Row = (Option<String>, u32);

/// The record, item and row identities a claim names, read loosely from any
/// object: a claim refused for its shape still names what it names.
#[derive(Default)]
struct Named {
    uuids: HashSet<String>,
    items: HashSet<String>,
    rows: HashSet<Row>,
}

impl Named {
    fn of(claim: Option<&Value>) -> Self {
        let mut named = Self::default();
        let Some(object) = claim.and_then(Value::as_object) else {
            return named;
        };
        let text = |key: &str| object.get(key).and_then(Value::as_str).map(str::to_owned);
        named.uuids.extend(text("record_uuid"));
        named.items.extend(text("item_id"));
        let rollout = object
            .get("segment")
            .and_then(|segment| segment.get("rollout_id"))
            .map(|rollout| rollout.as_str().map(str::to_owned));
        let index = object
            .get("row")
            .and_then(|row| row.get("index"))
            .and_then(small);
        if let Some(index) = index {
            // A row whose rollout is absent or unreadable may be any
            // rollout's, the flat one included.
            named.rows.insert((rollout.flatten(), index));
        }
        named
    }

    /// Whether `claim`'s identities were named here; a row named here with
    /// no readable rollout matches its index in every rollout.
    fn overlaps(&self, claim: &Self) -> bool {
        !self.uuids.is_disjoint(&claim.uuids)
            || !self.items.is_disjoint(&claim.items)
            || claim
                .rows
                .iter()
                .any(|row| self.rows.contains(row) || self.rows.contains(&(None, row.1)))
    }

    fn absorb(&mut self, other: Self) {
        self.uuids.extend(other.uuids);
        self.items.extend(other.items);
        self.rows.extend(other.rows);
    }
}

impl OriginSession {
    pub(super) fn open(marker: OriginMarker, native_session_id: &str) -> Self {
        Self {
            state: match marker {
                OriginMarker::Absent => SessionOrigin::Withheld,
                OriginMarker::V1 => SessionOrigin::Marked,
                OriginMarker::Invalid => SessionOrigin::Invalid,
            },
            native_session_id: native_session_id.to_owned(),
            seen: HashSet::new(),
            claimed: HashSet::new(),
            items: HashMap::new(),
            rows: HashMap::new(),
            indices: HashMap::new(),
            named: Named::default(),
            disputes: Vec::new(),
        }
    }

    pub(super) fn state(&self) -> SessionOrigin {
        self.state
    }

    /// The claimed UUIDs disputed since the last call, each named once per
    /// dispute. A claim is disputed by any later line of its session that
    /// repeats its record UUID, whatever that line claims, and by any other
    /// claim line, however malformed, naming its record, item or row. A claim
    /// that repeats what an earlier refused claim named is disputed as it is
    /// made. The per-line verdicts do not change.
    pub(super) fn take_disputes(&mut self) -> Vec<String> {
        std::mem::take(&mut self.disputes)
    }

    fn dispute(&mut self, named: &Named) {
        for uuid in &named.uuids {
            if self.claimed.contains(uuid) {
                self.disputes.push(uuid.clone());
            }
        }
        for item in &named.items {
            if let Some(uuid) = self.items.get(item) {
                self.disputes.push(uuid.clone());
            }
        }
        for row in &named.rows {
            match &row.0 {
                Some(_) => self.disputes.extend(self.rows.get(row).cloned()),
                // No readable rollout (a flat row, or one this claim does not
                // state): the index in every rollout.
                None => self
                    .disputes
                    .extend(self.indices.get(&row.1).into_iter().flatten().cloned()),
            }
        }
    }

    /// A line in a marked session that is not a parsed record (a record the
    /// parser dropped for want of an identity) may still name a claimed
    /// identity; it disputes it like any other line.
    pub(super) fn unparsed(&mut self, line: &str) {
        if self.state != SessionOrigin::Marked {
            return;
        }
        if let Ok(Probe {
            origin_evidence: Present(Some(Unrepeated(claim))),
        }) = serde_json::from_str::<Probe>(line)
        {
            let named = Named::of(claim.as_ref());
            self.dispute(&named);
            self.named.absorb(named);
        }
    }

    /// Judge the evidence on one record line whose canonical record parsed.
    pub(super) fn record(&mut self, line: &str, record: &CanonicalRecord) -> RecordOrigin {
        let claim = serde_json::from_str::<Probe>(line).map(|probe| probe.origin_evidence.0);
        match (self.state, claim) {
            (SessionOrigin::Invalid, _) => RecordOrigin::Invalid(OriginInvalid::SessionMarker),
            (SessionOrigin::Withheld, Err(_)) => RecordOrigin::Invalid(OriginInvalid::Shape),
            (SessionOrigin::Withheld, Ok(None)) => RecordOrigin::Withheld,
            (SessionOrigin::Withheld, Ok(Some(_))) => {
                RecordOrigin::Invalid(OriginInvalid::Unmarked)
            }
            (SessionOrigin::Marked, claim) => {
                // A parsed record always has a nonblank UUID.
                let uuid = record.uuid.clone().unwrap_or_default();
                if self.claimed.contains(&uuid) {
                    self.disputes.push(uuid.clone());
                }
                let origin = match claim {
                    Err(_) => RecordOrigin::Invalid(OriginInvalid::Shape),
                    Ok(None) if self.claimed.contains(&uuid) => {
                        RecordOrigin::Invalid(OriginInvalid::Duplicate)
                    }
                    Ok(None) => RecordOrigin::Declined,
                    Ok(Some(Unrepeated(claim))) => {
                        let named = Named::of(claim.as_ref());
                        self.dispute(&named);
                        let origin = match self.claim(claim.as_ref(), record, &uuid) {
                            Ok(proof) => {
                                if self.named.overlaps(&named) {
                                    self.disputes.push(uuid.clone());
                                }
                                RecordOrigin::Claimed(Box::new(proof))
                            }
                            Err(reason) => RecordOrigin::Invalid(reason),
                        };
                        self.named.absorb(named);
                        origin
                    }
                };
                self.seen.insert(uuid);
                origin
            }
        }
    }

    fn claim(
        &mut self,
        claim: Option<&Value>,
        record: &CanonicalRecord,
        uuid: &str,
    ) -> Result<InjectedContextProof, OriginInvalid> {
        let proof = closed_claim(claim.ok_or(OriginInvalid::Shape)?)?;
        if proof.native_session_id != self.native_session_id {
            return Err(OriginInvalid::SessionMismatch);
        }
        if proof.record_uuid != uuid {
            return Err(OriginInvalid::RecordMismatch);
        }
        if record.record_type != RecordType::User {
            return Err(OriginInvalid::NotUserRecord);
        }
        let row = (proof.rollout_id.clone(), proof.row_index);
        if self.seen.contains(uuid)
            || self.items.contains_key(&proof.item_id)
            || self.rows.contains_key(&row)
        {
            return Err(OriginInvalid::Duplicate);
        }
        self.claimed.insert(uuid.to_owned());
        self.items.insert(proof.item_id.clone(), uuid.to_owned());
        self.indices.entry(row.1).or_default().push(uuid.to_owned());
        self.rows.insert(row, uuid.to_owned());
        Ok(proof)
    }
}

/// Everything else on the line is skipped unread; a repeated
/// `origin_evidence` key fails the probe.
#[derive(Deserialize)]
struct Probe {
    #[serde(default)]
    origin_evidence: Present,
}

/// A present value, `null` included.
#[derive(Default)]
struct Present(Option<Unrepeated>);

impl<'de> Deserialize<'de> for Present {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Unrepeated::deserialize(deserializer).map(|value| Self(Some(value)))
    }
}

/// A JSON value, or `None` if an object anywhere in it repeats a key. A plain
/// `Value` keeps only the last of repeated keys, which would let a repeated
/// key pass a closed shape. The whole value is still consumed, so a repeat
/// fails the evidence, never the line.
struct Unrepeated(Option<Value>);

impl<'de> Deserialize<'de> for Unrepeated {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(UnrepeatedVisitor)
    }
}

struct UnrepeatedVisitor;

impl<'de> Visitor<'de> for UnrepeatedVisitor {
    type Value = Unrepeated;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Unrepeated, E> {
        Ok(Unrepeated(Some(Value::Bool(value))))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Unrepeated, E> {
        Ok(Unrepeated(Some(Value::from(value))))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Unrepeated, E> {
        Ok(Unrepeated(Some(Value::from(value))))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Unrepeated, E> {
        Ok(Unrepeated(Some(Value::from(value))))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Unrepeated, E> {
        Ok(Unrepeated(Some(Value::from(value))))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Unrepeated, E> {
        Ok(Unrepeated(Some(Value::String(value))))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Unrepeated, E> {
        Ok(Unrepeated(Some(Value::Null)))
    }

    fn visit_none<E: de::Error>(self) -> Result<Unrepeated, E> {
        Ok(Unrepeated(Some(Value::Null)))
    }

    fn visit_some<D: Deserializer<'de>>(self, deserializer: D) -> Result<Unrepeated, D::Error> {
        Unrepeated::deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Unrepeated, A::Error> {
        let mut values = Vec::new();
        let mut unrepeated = true;
        while let Some(Unrepeated(value)) = seq.next_element()? {
            match value {
                Some(value) => values.push(value),
                None => unrepeated = false,
            }
        }
        Ok(Unrepeated(unrepeated.then_some(Value::Array(values))))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Unrepeated, A::Error> {
        let mut object = Map::new();
        let mut unrepeated = true;
        while let Some(key) = map.next_key::<String>()? {
            let Unrepeated(value) = map.next_value()?;
            let fresh = value.is_some_and(|value| object.insert(key, value).is_none());
            unrepeated &= fresh;
        }
        Ok(Unrepeated(unrepeated.then_some(Value::Object(object))))
    }
}

/// The version 1 claim, checked key by key against the closed shape the Store
/// accepts. Identities only; nothing else is read.
fn closed_claim(claim: &Value) -> Result<InjectedContextProof, OriginInvalid> {
    let object = closed(
        claim,
        &[
            "contract",
            "version",
            "kind",
            "native_session_id",
            "segment",
            "row",
            "item_id",
            "turn_id",
            "record_uuid",
        ],
    )
    .ok_or(OriginInvalid::Shape)?;
    if object.get("contract").and_then(Value::as_str)
        != Some(OriginContract::CodexOriginEvidence.as_str())
    {
        return Err(OriginInvalid::Contract);
    }
    if !version_one(object.get("version")) {
        return Err(OriginInvalid::Version);
    }
    if object.get("kind").and_then(Value::as_str)
        != Some(InjectedContextKind::CodexSelectedSkillInstructions.as_str())
    {
        return Err(OriginInvalid::Kind);
    }
    let identifier = |key: &str, shape: fn(&str) -> bool| {
        object
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| shape(value))
            .map(str::to_owned)
            .ok_or(OriginInvalid::Identifier)
    };
    let native_session_id = identifier("native_session_id", canonical_uuid)?;
    let item_id = identifier("item_id", message_item_id)?;
    let turn_id = identifier("turn_id", canonical_uuid)?;
    let record_uuid = identifier("record_uuid", canonical_uuid)?;
    let segment = object
        .get("segment")
        .and_then(|segment| closed(segment, &["history", "rollout_id"]))
        .ok_or(OriginInvalid::Segment)?;
    let (history, rollout_id) = match (
        segment.get("history").and_then(Value::as_str),
        segment.get("rollout_id"),
    ) {
        (Some("flat"), Some(Value::Null)) => (SegmentHistory::Flat, None),
        (Some("paginated"), Some(Value::String(rollout))) if canonical_uuid(rollout) => {
            (SegmentHistory::Paginated, Some(rollout.clone()))
        }
        _ => return Err(OriginInvalid::Segment),
    };
    let row = object
        .get("row")
        .and_then(|row| closed(row, &["index", "ordinal"]))
        .ok_or(OriginInvalid::Row)?;
    let row_index = row.get("index").and_then(small).ok_or(OriginInvalid::Row)?;
    let row_ordinal = match row.get("ordinal") {
        Some(Value::Null) => None,
        Some(ordinal) => Some(small(ordinal).ok_or(OriginInvalid::Row)?),
        None => return Err(OriginInvalid::Row),
    };
    Ok(InjectedContextProof {
        contract: OriginContract::CodexOriginEvidence,
        version: ORIGIN_EVIDENCE_VERSION,
        kind: InjectedContextKind::CodexSelectedSkillInstructions,
        native_session_id,
        history,
        rollout_id,
        row_index,
        row_ordinal,
        item_id,
        turn_id,
        record_uuid,
    })
}

/// An object holding exactly these keys.
fn closed<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Map<String, Value>> {
    value.as_object().filter(|object| {
        object.len() == keys.len() && keys.iter().all(|key| object.contains_key(*key))
    })
}

/// The JSON integer 1: not `1.0`, `"1"` or `true`.
fn version_one(value: Option<&Value>) -> bool {
    value.and_then(Value::as_u64) == Some(u64::from(ORIGIN_EVIDENCE_VERSION))
}

/// A nonnegative JSON integer the Store's row columns hold.
fn small(value: &Value) -> Option<u32> {
    value.as_u64().and_then(|value| u32::try_from(value).ok())
}

/// A canonical lowercase UUID, as the Store checks it.
fn canonical_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => matches!(byte, b'0'..=b'9' | b'a'..=b'f'),
        })
}

/// A native Codex message item ID, as the Store checks it: `msg_` followed by
/// a canonical UUID or 50 lowercase hex digits.
fn message_item_id(value: &str) -> bool {
    value.strip_prefix("msg_").is_some_and(|rest| {
        canonical_uuid(rest)
            || (rest.len() == 50 && rest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
    })
}
