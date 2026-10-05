//! The wire contract for opening one session's original local source.
//!
//! The detail view asks for a session's own text on demand. This type is what
//! comes back: either that the source loaded, with everything needed to say
//! how complete it is, or the one reason it may not be shown, while the saved
//! measurements beside it stay exactly as they are.
//!
//! Two things are deliberately absent. **Local paths**, as in
//! [`crate::dto::NativeIndexStatus`]: where a session's history lives is index
//! metadata, not view data. **Diagnostic text of any kind** — no failure
//! message, no producer output: every state here is a closed value the view
//! can render on its own terms, and the local detail stays local.
//!
//! The transcript itself is here now. It was held back until the components
//! that render it existed, so that their props could define its shape rather
//! than a second shape being invented ahead of them; [`crate::transcript_dto`]
//! is that translation. `records` is the session's own records, in read order.
//! A count would have been a second truth about the same list, so the list is
//! the only one.
use serde::Serialize;
use ts_rs::TS;
use xt_ingest::native::readers_cli::{DetailFailure, DetailLimit, DetailRefusal};
use xt_ingest::native::session_source::{
    GenerationBasis, SessionSourceOutcome, SourceCeiling, SourceGap, SourceUnavailable,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionSourceStatus {
    Loaded {
        generation: SessionGeneration,
        /// Files of this session that were read: its own transcript, and any
        /// subagent transcripts filed under it.
        sources: u32,
        /// This session's records, in the order they were read: its own
        /// transcript, then each subagent transcript filed under it.
        records: Vec<crate::transcript_dto::SourceRecord>,
        /// Records the canonical parser accepted but could not identify. They
        /// are counted, never shown as turns.
        dropped_records: u32,
        /// Everything this read could not cover. An empty list is the only
        /// claim that the text is the whole session.
        gaps: Vec<SessionSourceGap>,
    },
    /// The text may not be shown. The session's measurements are unaffected:
    /// this says nothing about whether the work happened.
    Unavailable { reason: SessionSourceReason },
}

/// How the text relates to the generation the saved measurements were taken
/// from, over every file of the session together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "generation", rename_all = "snake_case")]
pub enum SessionGeneration {
    /// The measured bytes are still the first bytes of every file.
    /// `appended` means the session has grown since it was measured.
    Indexed { appended: bool },
    /// At least one file has no recorded checkpoint, so the text cannot be
    /// tied to the measurement. It is still this session's own source.
    Unrecorded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "gap", rename_all = "snake_case")]
pub enum SessionSourceGap {
    /// A file of this session could not be opened or read.
    Unreadable,
    /// A file of this session is no longer the generation that was measured,
    /// or it changed while it was being read.
    Replaced,
    /// A file stopped at a line that does not meet the canonical contract,
    /// exactly where indexing stopped too.
    Stopped {
        #[ts(type = "number")]
        line: u64,
    },
    /// Part of this session's own subagent tree could not be listed, so its
    /// set of files may be incomplete. What was read is still this session's.
    DiscoveryIncomplete,
    /// A subagent transcript the index read is no longer there. Its records
    /// are inside the saved measurements, so the text is missing a part the
    /// numbers still count.
    Missing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum SessionSourceLimit {
    Bytes,
    Records,
}

/// Why a session's original source may not be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum SessionSourceReason {
    /// Nothing under this machine's history names the session any more.
    Missing,
    /// The source is no longer where it was read from. It is not followed to
    /// its new place: the measurements belong to the place it was read.
    Moved,
    /// The source is no longer the generation that was measured, or it changed
    /// while it was being read.
    Replaced,
    /// The source, or the history that would hold it, could not be read.
    Unreadable,
    /// More than one source names this session; showing either could show the
    /// wrong one.
    Ambiguous { candidates: u32 },
    /// `reached` and `ceiling` are counted in whatever `limit` names, so a
    /// record ceiling is never reported as a byte figure.
    TooLarge {
        limit: SessionSourceLimit,
        #[ts(type = "number")]
        reached: u64,
        #[ts(type = "number")]
        ceiling: u64,
    },
    /// The read was cancelled; no part of the text is returned.
    Cancelled,
    /// The identifier could not name one session.
    InvalidIdentifier,
    /// This machine's index holds no local source identity for this session,
    /// so there is nothing to reopen: it names no session here, it was
    /// recorded without one, or this app is not reading a local history at
    /// all. Whatever was measured about it stands; only the text is absent.
    NotIndexed,
    /// This host's sources are only reachable through the pinned reader, and
    /// the read was not given one. The app always gives Codex and Cursor reads
    /// one or says why it could not ([`Self::ReaderUnavailable`]), so this is
    /// kept for callers without a reader.
    PrerequisiteUnavailable,
    /// No native reader exists for this host at all.
    UnsupportedHost,
    /// The pinned reader could not be run for this open. Nothing was read.
    ReaderUnavailable { cause: ReaderUnavailableCause },
    /// The session is larger than the pinned reader reads at once, by the
    /// ceiling named. It is refused whole, never shown in part.
    ReaderLimit { limit: ReaderLimit },
    /// The pinned reader did not finish within its deadline; it was stopped
    /// and nothing it read was kept.
    ReaderDeadline,
    /// The session's selected Cursor source is a SQLite store, which the
    /// pinned reader does not read for a transcript. Its other files are never
    /// read in its place.
    StoreUnsupported,
    /// The pinned reader's answer was not exactly this session, whole; none
    /// of it was kept.
    ReaderProtocol,
}

/// Why the pinned reader could not be run for an open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReaderUnavailableCause {
    /// No Python 3.10+ interpreter could be found, or it could not be started.
    Interpreter,
    /// The bundled reader files are not exactly the pinned ones.
    Readers,
    /// This app is not reading local history (the index is disabled).
    Index,
}

/// One of the pinned reader's declared ceilings for reading one session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ReaderLimit {
    SourceBytes,
    Files,
    NativeRows,
    Records,
    LineBytes,
    OutputBytes,
    HeaderBytes,
    DiscoveryEntries,
    /// Identifying which file holds the session, not the session's size: one
    /// discovered file's identity probe reached its ceiling.
    HeaderProbeBytes,
    /// Identifying: all identity probes together reached their ceiling.
    ProbeBytes,
    /// Identifying: more discovered files than may be probed.
    Probes,
}

/// Counts cross into JSON as exact integers or not at all.
fn count(value: usize) -> Result<u32, &'static str> {
    u32::try_from(value).map_err(|_| "session source count exceeds the reported range")
}

fn reader_limit(limit: DetailLimit) -> ReaderLimit {
    match limit {
        DetailLimit::SourceBytes => ReaderLimit::SourceBytes,
        DetailLimit::Files => ReaderLimit::Files,
        DetailLimit::NativeRows => ReaderLimit::NativeRows,
        DetailLimit::Records => ReaderLimit::Records,
        DetailLimit::LineBytes => ReaderLimit::LineBytes,
        DetailLimit::OutputBytes => ReaderLimit::OutputBytes,
        DetailLimit::HeaderBytes => ReaderLimit::HeaderBytes,
        DetailLimit::DiscoveryEntries => ReaderLimit::DiscoveryEntries,
        DetailLimit::HeaderProbeBytes => ReaderLimit::HeaderProbeBytes,
        DetailLimit::ProbeBytes => ReaderLimit::ProbeBytes,
        DetailLimit::Probes => ReaderLimit::Probes,
    }
}

/// A pinned reader's refusal, as a closed reason. No producer text, code or
/// path crosses: the reason is chosen from the failure's kind alone.
fn reader_reason(failure: DetailFailure) -> SessionSourceReason {
    match failure {
        DetailFailure::Cancelled => SessionSourceReason::Cancelled,
        DetailFailure::Start => SessionSourceReason::ReaderUnavailable {
            cause: ReaderUnavailableCause::Interpreter,
        },
        DetailFailure::Deadline | DetailFailure::Refused(DetailRefusal::Deadline) => {
            SessionSourceReason::ReaderDeadline
        }
        DetailFailure::Refused(DetailRefusal::Limit(limit)) => SessionSourceReason::ReaderLimit {
            limit: reader_limit(limit),
        },
        DetailFailure::Refused(DetailRefusal::StoreUnsupported) => {
            SessionSourceReason::StoreUnsupported
        }
        DetailFailure::Refused(DetailRefusal::SourceChanged) => SessionSourceReason::Replaced,
        // The producer says so when nothing under this home names the session.
        DetailFailure::Refused(DetailRefusal::Unavailable) => SessionSourceReason::Missing,
        DetailFailure::Refused(DetailRefusal::Unreadable | DetailRefusal::DiscoveryIncomplete) => {
            SessionSourceReason::Unreadable
        }
        // More output than the producer's own ceiling allows is a broken
        // contract, as is anything else that is not one whole session.
        DetailFailure::OutputBound | DetailFailure::Protocol(_) => {
            SessionSourceReason::ReaderProtocol
        }
    }
}

fn limit(ceiling: SourceCeiling) -> SessionSourceLimit {
    match ceiling {
        SourceCeiling::Bytes => SessionSourceLimit::Bytes,
        SourceCeiling::Records => SessionSourceLimit::Records,
    }
}

impl TryFrom<&SessionSourceOutcome> for SessionSourceStatus {
    type Error = &'static str;

    fn try_from(outcome: &SessionSourceOutcome) -> Result<Self, Self::Error> {
        Ok(match outcome {
            SessionSourceOutcome::Loaded(session) => Self::Loaded {
                generation: match session.generation {
                    GenerationBasis::Indexed { appended } => {
                        SessionGeneration::Indexed { appended }
                    }
                    GenerationBasis::Unrecorded => SessionGeneration::Unrecorded,
                },
                sources: count(session.sources.len())?,
                records: crate::transcript_dto::records(&session.records)?,
                dropped_records: count(
                    usize::try_from(session.dropped_records).unwrap_or(usize::MAX),
                )?,
                gaps: session
                    .gaps
                    .iter()
                    .map(|gap| match gap {
                        // A gap names that something is missing from the text.
                        // Which file it was in is local history metadata.
                        SourceGap::Unreadable { .. } => SessionSourceGap::Unreadable,
                        SourceGap::Replaced { .. } => SessionSourceGap::Replaced,
                        SourceGap::Stopped { line, .. } => {
                            SessionSourceGap::Stopped { line: *line }
                        }
                        SourceGap::DiscoveryIncomplete => SessionSourceGap::DiscoveryIncomplete,
                        SourceGap::Missing { .. } => SessionSourceGap::Missing,
                    })
                    .collect(),
            },
            SessionSourceOutcome::Unavailable(reason) => Self::Unavailable {
                reason: match reason {
                    SourceUnavailable::Missing => SessionSourceReason::Missing,
                    // The place a moved source was found is local history
                    // metadata; the view is told that it moved, not where to.
                    SourceUnavailable::Moved { .. } => SessionSourceReason::Moved,
                    SourceUnavailable::Replaced { .. } => SessionSourceReason::Replaced,
                    // The failure kind stays local: it is a diagnostic, and a
                    // diagnostic is exactly the kind of free text that must
                    // not carry a path or a fragment of a transcript.
                    SourceUnavailable::Unreadable { .. } => SessionSourceReason::Unreadable,
                    SourceUnavailable::Ambiguous { candidates } => SessionSourceReason::Ambiguous {
                        candidates: count(*candidates)?,
                    },
                    SourceUnavailable::TooLarge {
                        limit: ceiling,
                        reached,
                        ceiling: bound,
                    } => SessionSourceReason::TooLarge {
                        limit: limit(*ceiling),
                        reached: *reached,
                        ceiling: *bound,
                    },
                    SourceUnavailable::Cancelled => SessionSourceReason::Cancelled,
                    SourceUnavailable::InvalidIdentifier => SessionSourceReason::InvalidIdentifier,
                    SourceUnavailable::PrerequisiteUnavailable => {
                        SessionSourceReason::PrerequisiteUnavailable
                    }
                    SourceUnavailable::UnsupportedHost => SessionSourceReason::UnsupportedHost,
                    SourceUnavailable::Reader(failure) => reader_reason(*failure),
                },
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use xt_ingest::native::{
        checkpoint::ResumeBasis,
        session_source::{LoadedSession, ReadSource, SourceRole},
    };
    use xt_store::Host;

    fn status(outcome: SessionSourceOutcome) -> SessionSourceStatus {
        SessionSourceStatus::try_from(&outcome).unwrap()
    }

    fn json(outcome: SessionSourceOutcome) -> String {
        serde_json::to_string(&status(outcome)).unwrap()
    }

    fn reason(outcome: SourceUnavailable) -> SessionSourceReason {
        match status(SessionSourceOutcome::Unavailable(outcome)) {
            SessionSourceStatus::Unavailable { reason } => reason,
            other => panic!("{other:?}"),
        }
    }

    /// One canonical line, parsed the way the loader parses one.
    fn parsed(line: serde_json::Value) -> xt_ingest::canonical::ParsedRecord {
        match xt_ingest::canonical::parse_line(&line.to_string()).unwrap() {
            xt_ingest::canonical::Parsed::Record(record) => *record,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_loaded_session_carries_its_records_and_still_names_no_place() {
        let wire = json(SessionSourceOutcome::Loaded(Box::new(LoadedSession {
            native_session_id: "00000000-0000-4000-8000-00000000aaaa".into(),
            host: Host::Claude,
            generation: GenerationBasis::Unrecorded,
            sources: vec![ReadSource {
                path: PathBuf::from("/home/user/.claude/projects/-repo/session.jsonl"),
                role: SourceRole::Primary,
                bytes: 512,
            }],
            records: vec![parsed(serde_json::json!({
                "uuid": "11111111-1111-4111-8111-111111111111",
                "type": "assistant",
                "timestamp": "2026-09-07T12:00:00.000Z",
                "message": {"role": "assistant", "content": [
                    {"type": "text", "text": "reading it"},
                    {"type": "tool_use", "id": "toolu_1", "name": "Read",
                     "input": {"file_path": "/repo/a.txt"}}
                ]}
            }))],
            other_lines: 0,
            dropped_records: 0,
            gaps: Vec::new(),
        })));
        assert_eq!(
            wire,
            r#"{"state":"loaded","generation":{"generation":"unrecorded"},"sources":1,"records":[{"id":"11111111-1111-4111-8111-111111111111","role":"assistant","at":"2026-09-07T12:00:00.000Z","blocks":[{"kind":"text","index":0,"text":"reading it"},{"kind":"tool_call","index":1,"name":"Read","call_id":"toolu_1","input":{"kind":"json","value":{"file_path":"/repo/a.txt"}}}]}],"dropped_records":0,"gaps":[]}"#
        );
        // The transcript crosses; where it lives does not.
        assert!(!wire.contains(".claude"), "{wire}");
    }

    /// Every way the pinned reader can refuse crosses as one closed reason:
    /// each declared ceiling by name, the deadline, the SQLite store, and a
    /// broken protocol — never a loaded state, and never the producer's text.
    #[test]
    fn a_reader_refusal_crosses_as_its_own_closed_reason() {
        use DetailFailure as F;
        use DetailRefusal as R;
        for (failure, expected) in [
            (F::Cancelled, SessionSourceReason::Cancelled),
            (
                F::Start,
                SessionSourceReason::ReaderUnavailable {
                    cause: ReaderUnavailableCause::Interpreter,
                },
            ),
            (F::Deadline, SessionSourceReason::ReaderDeadline),
            (F::Refused(R::Deadline), SessionSourceReason::ReaderDeadline),
            (
                F::Refused(R::StoreUnsupported),
                SessionSourceReason::StoreUnsupported,
            ),
            (F::Refused(R::SourceChanged), SessionSourceReason::Replaced),
            (F::Refused(R::Unavailable), SessionSourceReason::Missing),
            (F::Refused(R::Unreadable), SessionSourceReason::Unreadable),
            (
                F::Refused(R::DiscoveryIncomplete),
                SessionSourceReason::Unreadable,
            ),
            (F::OutputBound, SessionSourceReason::ReaderProtocol),
            (
                F::Protocol("session source is outside this host's roots"),
                SessionSourceReason::ReaderProtocol,
            ),
            (
                F::Refused(R::Limit(DetailLimit::NativeRows)),
                SessionSourceReason::ReaderLimit {
                    limit: ReaderLimit::NativeRows,
                },
            ),
        ] {
            assert_eq!(
                reason(SourceUnavailable::Reader(failure)),
                expected,
                "{failure:?}"
            );
        }
        let wire = json(SessionSourceOutcome::Unavailable(
            SourceUnavailable::Reader(F::Protocol("session source is outside this host's roots")),
        ));
        assert_eq!(
            wire,
            r#"{"state":"unavailable","reason":{"reason":"reader_protocol"}}"#
        );
        let wire = json(SessionSourceOutcome::Unavailable(
            SourceUnavailable::Reader(F::Refused(R::Limit(DetailLimit::SourceBytes))),
        ));
        assert_eq!(
            wire,
            r#"{"state":"unavailable","reason":{"reason":"reader_limit","limit":"source_bytes"}}"#
        );
        // Identification ceilings keep their own names: a history too large to
        // search is neither the selected session's size nor a broken answer.
        for (limit, name) in [
            (DetailLimit::HeaderProbeBytes, "header_probe_bytes"),
            (DetailLimit::ProbeBytes, "probe_bytes"),
            (DetailLimit::Probes, "probes"),
        ] {
            let wire = json(SessionSourceOutcome::Unavailable(
                SourceUnavailable::Reader(F::Refused(R::Limit(limit))),
            ));
            assert_eq!(
                wire,
                format!(
                    r#"{{"state":"unavailable","reason":{{"reason":"reader_limit","limit":"{name}"}}}}"#
                )
            );
        }
    }

    #[test]
    fn a_moved_source_says_so_without_naming_where_it_lives() {
        let wire = json(SessionSourceOutcome::Unavailable(
            SourceUnavailable::Moved {
                from: PathBuf::from("/home/user/.claude/projects/-secret-repo/session.jsonl"),
                found: PathBuf::from("/home/user/.claude/projects/-renamed/session.jsonl"),
            },
        ));
        assert_eq!(
            wire,
            r#"{"state":"unavailable","reason":{"reason":"moved"}}"#
        );
        assert!(!wire.contains("secret"), "{wire}");
        assert!(!wire.contains(".claude"), "{wire}");
    }

    #[test]
    fn an_unreadable_source_carries_no_diagnostic_text() {
        // The local detail names a failure kind, and could one day name more;
        // none of it crosses, so nothing here can carry a path or a fragment
        // of what the session said.
        let wire = json(SessionSourceOutcome::Unavailable(
            SourceUnavailable::Unreadable {
                detail: "source could not be read: PermissionDenied".into(),
            },
        ));
        assert_eq!(
            wire,
            r#"{"state":"unavailable","reason":{"reason":"unreadable"}}"#
        );
    }

    #[test]
    fn every_unavailable_reason_reaches_the_view_as_its_own_state() {
        for (outcome, expected) in [
            (SourceUnavailable::Missing, SessionSourceReason::Missing),
            (
                SourceUnavailable::Replaced {
                    basis: ResumeBasis::Truncated,
                },
                SessionSourceReason::Replaced,
            ),
            (
                SourceUnavailable::Ambiguous { candidates: 2 },
                SessionSourceReason::Ambiguous { candidates: 2 },
            ),
            (
                SourceUnavailable::TooLarge {
                    limit: SourceCeiling::Records,
                    reached: 200_001,
                    ceiling: 200_000,
                },
                // A record ceiling reports records, never a byte figure.
                SessionSourceReason::TooLarge {
                    limit: SessionSourceLimit::Records,
                    reached: 200_001,
                    ceiling: 200_000,
                },
            ),
            (SourceUnavailable::Cancelled, SessionSourceReason::Cancelled),
            (
                SourceUnavailable::InvalidIdentifier,
                SessionSourceReason::InvalidIdentifier,
            ),
            (
                SourceUnavailable::PrerequisiteUnavailable,
                SessionSourceReason::PrerequisiteUnavailable,
            ),
            (
                SourceUnavailable::UnsupportedHost,
                SessionSourceReason::UnsupportedHost,
            ),
        ] {
            assert_eq!(reason(outcome), expected);
        }
    }

    #[test]
    fn a_loaded_session_reports_what_it_could_not_cover() {
        let wire = json(SessionSourceOutcome::Loaded(Box::new(LoadedSession {
            native_session_id: "00000000-0000-4000-8000-00000000aaaa".into(),
            host: Host::Claude,
            generation: GenerationBasis::Indexed { appended: true },
            sources: vec![ReadSource {
                path: PathBuf::from("/home/user/.claude/projects/-repo/session.jsonl"),
                role: SourceRole::Primary,
                bytes: 1024,
            }],
            records: Vec::new(),
            other_lines: 3,
            dropped_records: 2,
            gaps: vec![
                SourceGap::DiscoveryIncomplete,
                SourceGap::Missing {
                    path: PathBuf::from("/home/user/.claude/projects/-repo/gone/subagents/a.jsonl"),
                },
                SourceGap::Stopped {
                    path: PathBuf::from("/home/user/.claude/projects/-repo/session.jsonl"),
                    line: 41,
                    reason: "transcript is not UTF-8",
                },
            ],
        })));
        assert_eq!(
            wire,
            r#"{"state":"loaded","generation":{"generation":"indexed","appended":true},"sources":1,"records":[],"dropped_records":2,"gaps":[{"gap":"discovery_incomplete"},{"gap":"missing"},{"gap":"stopped","line":41}]}"#
        );
        // A gap names that something is missing, never the file it is in.
        assert!(!wire.contains(".claude"), "{wire}");
    }
}
