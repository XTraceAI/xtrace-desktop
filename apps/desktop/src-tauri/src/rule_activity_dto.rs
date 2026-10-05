//! The wire contract of recorded rule activity.
//!
//! One read is one bounded snapshot of the default local rulebook's ledger,
//! reduced to structural metadata. It describes rows *recorded* in that one
//! source, never all rule activity: an empty snapshot is "0 recorded rows",
//! not "no rules fired", and a recorded mode is a delivery mode, never an
//! outcome (`gate` does not mean blocked, `advise` does not mean followed,
//! `suppressed` was not delivered).
//!
//! What never crosses this boundary: the source's path or root, file
//! identity, raw JSON, `excerpt`, `override_reason`, `dedup_key`,
//! `source_message_id`, matcher counts, parser or filesystem error text, and
//! the recorded agent, worktree, repository and branch, which no view needs
//! yet. Every state other than `loaded` carries no counts, so a failed,
//! changed or interrupted read can never be shown as a stale or zero one.
use serde::Serialize;
use ts_rs::TS;
use xt_rulebook::activity::{
    ActivityRead, ActivitySnapshot, ActivityWindow, FireMode, FireRecord, Interruption, ModeCounts,
    ObservedGroup, PathProblem, Precision, RuleVersion, SourceChange, SourcePart, Unavailable,
};

/// The one source this reads: the default local rulebook ("Default local
/// rulebook source"). Its location on disk stays native-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuleActivitySource {
    DefaultLocalRulebook,
}

/// The outcome of one `rule_activity_read`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum RuleActivityResult {
    /// A snapshot was read. A valid empty source is a real `loaded` state
    /// with zero rows.
    Loaded(Box<RuleActivityLoaded>),
    /// No usable source; never an empty one.
    Unavailable {
        source: RuleActivitySource,
        part: RuleActivityPart,
        reason: RuleActivityUnavailableReason,
        /// The schema the marker names, as a decimal string, only when
        /// `reason` is `unsupported_schema`.
        found_version: Option<String>,
    },
    /// The ledger was replaced, shrunk or removed while it was read. Nothing
    /// from the read is kept; only an explicit retry reads again.
    SourceChanged {
        source: RuleActivitySource,
        change: RuleActivityChange,
    },
    /// Cancelled or past the read's deadline; nothing from it is kept.
    Interrupted { reason: RuleActivityInterruption },
    /// Another read is running, or this one already is. Nothing was read and
    /// the running read is untouched.
    Busy { reason: RuleActivityBusy },
    /// The app is shutting down; nothing was read.
    Closed,
    /// The read failed in a way with no safe detail to report.
    Failed,
    /// The read identifier is empty, too long or not an opaque token. It is
    /// never echoed.
    InvalidReadId,
}

/// Which path of the source a problem concerns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuleActivityPart {
    Root,
    LedgerDir,
    SchemaMarker,
    Ledger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuleActivityUnavailableReason {
    Missing,
    Unreadable,
    /// A link, a non-regular file, a shared (hard-linked) file or a path
    /// outside the source root; it is not read.
    UnsafePath,
    MalformedSchema,
    UnsupportedSchema,
    /// This process has no source to read: fixture mode, which never falls
    /// back to the user's own home.
    NotConfigured,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuleActivityChange {
    Replaced,
    Shrunk,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuleActivityInterruption {
    Cancelled,
    Deadline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuleActivityBusy {
    /// A read under another identifier is running.
    AnotherRead,
    /// A read under this identifier is already running.
    DuplicateRead,
}

/// One bounded snapshot of recorded rule activity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct RuleActivityLoaded {
    /// The identifier this read was requested under.
    pub read_id: String,
    pub source: RuleActivitySource,
    pub schema_version: u32,
    /// RFC 3339 instants. The window is the fixed trailing 14 × 24 hours
    /// `[window_start, window_end)`, anchored once when the read was admitted.
    pub read_at: String,
    pub window_start: String,
    pub window_end: String,
    pub counts: RuleActivityCounts,
    pub coverage: RuleActivityCoverage,
    /// Latest in-window recorded rows, newest first; at most 100.
    pub latest_fires: Vec<RuleActivityFire>,
    /// More in-window rows were accepted than `latest_fires` holds. The
    /// counts still cover all of them.
    pub fires_truncated: bool,
    /// Observed `(rulebook, rule)` groups of *every* accepted in-window row,
    /// latest observed first; at most 100. Not active or distinct rules.
    pub observed_groups: Vec<RuleActivityGroup>,
    /// Every observed group, including those past the presentation bound.
    #[ts(type = "number")]
    pub observed_group_count: u64,
    /// More groups were observed than `observed_groups` holds. Returned
    /// groups keep their full counts.
    pub groups_truncated: bool,
}

/// Counts of *retained recorded source rows* in this snapshot, never of all
/// rule activity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct RuleActivityCounts {
    /// `exact` only for a complete, clean scan of the whole ledger; any bound,
    /// malformed row, conflict or unfinished line makes every count here,
    /// and every group's, a lower bound.
    pub precision: RuleActivityPrecision,
    /// Quality diagnostics over every examined line, in or out of the window.
    pub snapshot: RuleActivitySnapshotCounts,
    /// Distinct consistent records inside the window, by recorded mode.
    pub window_modes: RuleActivityModeCounts,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RuleActivityPrecision {
    Exact,
    LowerBound,
}

/// Snapshot-wide diagnostics; not window counts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct RuleActivitySnapshotCounts {
    /// Complete physical lines examined.
    #[ts(type = "number")]
    pub lines: u64,
    #[ts(type = "number")]
    pub blank_lines: u64,
    /// Rows that passed validation, before deduplication.
    #[ts(type = "number")]
    pub valid_rows: u64,
    /// Distinct fire identifiers with consistent metadata.
    #[ts(type = "number")]
    pub distinct_ids: u64,
    #[ts(type = "number")]
    pub duplicate_rows: u64,
    /// Fire identifiers recorded with differing metadata; excluded from
    /// every row, mode count and group.
    #[ts(type = "number")]
    pub conflicted_ids: u64,
    #[ts(type = "number")]
    pub conflicted_rows: u64,
    pub malformed: RuleActivityMalformed,
}

/// Rejected lines, by reason. No line content is kept.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct RuleActivityMalformed {
    #[ts(type = "number")]
    pub oversize_line: u64,
    #[ts(type = "number")]
    pub invalid_json: u64,
    #[ts(type = "number")]
    pub invalid_shape: u64,
    #[ts(type = "number")]
    pub invalid_timestamp: u64,
    #[ts(type = "number")]
    pub oversize_value: u64,
}

/// Distinct consistent records by recorded mode. A mode is not an outcome;
/// an unrecognized mode is counted apart and mapped to none of the others.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct RuleActivityModeCounts {
    #[ts(type = "number")]
    pub advise: u64,
    #[ts(type = "number")]
    pub gate: u64,
    #[ts(type = "number")]
    pub suppressed: u64,
    #[ts(type = "number")]
    pub unrecognized: u64,
}

/// What part of the ledger the snapshot covers. Byte offsets are decimal
/// strings, so no JavaScript number ever rounds them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct RuleActivityCoverage {
    /// Ledger size captured at open; nothing past it was read.
    pub captured_len: String,
    /// The byte range `[scanned_start, scanned_end)` of the complete lines
    /// examined.
    pub scanned_start: String,
    pub scanned_end: String,
    pub byte_bound_reached: bool,
    pub line_bound_reached: bool,
    pub leading_partial_dropped: bool,
    pub trailing_partial_dropped: bool,
    /// The ledger grew after capture; those rows are newer than this read.
    pub grew_after_capture: bool,
    /// Earliest accepted recorded instant. Not a coverage start: the ledger
    /// promises no retention.
    pub oldest_observed: Option<String>,
    pub newest_observed: Option<String>,
}

/// One recorded row, as structural metadata. Identities are the raw
/// recorded values; `host` and `session_id` are not linked to any indexed
/// session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct RuleActivityFire {
    pub fire_id: String,
    pub rule_id: String,
    pub rule_version: Option<RuleActivityVersion>,
    /// `null` when the row names no rulebook: unscoped, not "any rulebook".
    pub rulebook_id: Option<String>,
    pub fired_at: String,
    pub tool: Option<String>,
    pub hook_phase: Option<String>,
    pub host: Option<String>,
    /// The host's own session identifier, as recorded.
    pub session_id: String,
    pub mode: RuleActivityMode,
}

/// A recorded rule version: an integer (as a decimal string, so it is never
/// rounded) or a label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleActivityVersion {
    Number { value: String },
    Label { value: String },
}

/// The recorded delivery mode of a row. It is not an outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RuleActivityMode {
    Advise,
    Gate,
    Suppressed,
    /// Any other recorded value, verbatim, never mapped to a known mode.
    Unrecognized {
        value: String,
    },
}

/// The in-window rows of one observed `(rulebook, rule)` pair. A group
/// spans every recorded mode and version and names no current mode,
/// version, outcome or lifecycle state. An unscoped group (`rulebook_id`
/// `null`) never merges with a scoped one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct RuleActivityGroup {
    pub rulebook_id: Option<String>,
    pub rule_id: String,
    pub latest_observed: String,
    pub modes: RuleActivityModeCounts,
}

/// The largest integer a JSON number carries exactly into JavaScript.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// A count that a JavaScript number would not carry exactly. Every count is
/// bounded far below this by the read's own limits; the check makes that a
/// property of the boundary rather than an assumption.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Unrepresentable;

fn count(value: u64) -> Result<u64, Unrepresentable> {
    if value > MAX_SAFE_INTEGER {
        return Err(Unrepresentable);
    }
    Ok(value)
}

impl RuleActivityResult {
    /// The fixed answer when this process has no source to read.
    pub(crate) fn not_configured() -> Self {
        Self::Unavailable {
            source: RuleActivitySource::DefaultLocalRulebook,
            part: RuleActivityPart::Root,
            reason: RuleActivityUnavailableReason::NotConfigured,
            found_version: None,
        }
    }

    /// A reader's outcome on the wire. A snapshot that cannot be represented
    /// exactly is `failed`, never a rounded one.
    pub(crate) fn from_read(read_id: &str, window: ActivityWindow, read: ActivityRead) -> Self {
        let source = RuleActivitySource::DefaultLocalRulebook;
        match read {
            ActivityRead::Snapshot(snapshot) => match loaded(read_id, window, &snapshot) {
                Ok(loaded) => Self::Loaded(Box::new(loaded)),
                Err(Unrepresentable) => Self::Failed,
            },
            ActivityRead::Unavailable(unavailable) => {
                let (part, reason, found_version) = match unavailable {
                    Unavailable::RootNotAbsolute => (
                        RuleActivityPart::Root,
                        RuleActivityUnavailableReason::UnsafePath,
                        None,
                    ),
                    Unavailable::Path { part, problem } => (part.into(), problem.into(), None),
                    Unavailable::SchemaMalformed => (
                        RuleActivityPart::SchemaMarker,
                        RuleActivityUnavailableReason::MalformedSchema,
                        None,
                    ),
                    Unavailable::SchemaUnsupported { found } => (
                        RuleActivityPart::SchemaMarker,
                        RuleActivityUnavailableReason::UnsupportedSchema,
                        Some(found.to_string()),
                    ),
                };
                Self::Unavailable {
                    source,
                    part,
                    reason,
                    found_version,
                }
            }
            ActivityRead::SourceChanged(change) => Self::SourceChanged {
                source,
                change: match change {
                    SourceChange::Replaced => RuleActivityChange::Replaced,
                    SourceChange::Shrunk => RuleActivityChange::Shrunk,
                    SourceChange::Removed => RuleActivityChange::Removed,
                },
            },
            ActivityRead::Interrupted(why) => Self::Interrupted {
                reason: match why {
                    Interruption::Cancelled => RuleActivityInterruption::Cancelled,
                    Interruption::DeadlineExceeded => RuleActivityInterruption::Deadline,
                },
            },
        }
    }
}

impl From<SourcePart> for RuleActivityPart {
    fn from(part: SourcePart) -> Self {
        match part {
            SourcePart::Root => Self::Root,
            SourcePart::LedgerDir => Self::LedgerDir,
            SourcePart::SchemaMarker => Self::SchemaMarker,
            SourcePart::Ledger => Self::Ledger,
        }
    }
}

impl From<PathProblem> for RuleActivityUnavailableReason {
    fn from(problem: PathProblem) -> Self {
        match problem {
            PathProblem::Missing => Self::Missing,
            // The object opened was not the one located: it moved in between,
            // so it could not be read as located. The I/O kind stays local.
            PathProblem::Changed | PathProblem::Unreadable(_) => Self::Unreadable,
            PathProblem::Symlink
            | PathProblem::NotDirectory
            | PathProblem::NotRegularFile
            | PathProblem::MultipleLinks
            | PathProblem::OutsideRoot => Self::UnsafePath,
        }
    }
}

fn loaded(
    read_id: &str,
    window: ActivityWindow,
    snapshot: &ActivitySnapshot,
) -> Result<RuleActivityLoaded, Unrepresentable> {
    let counts = &snapshot.counts;
    let coverage = &snapshot.coverage;
    let malformed = &counts.malformed;
    Ok(RuleActivityLoaded {
        read_id: read_id.to_owned(),
        source: RuleActivitySource::DefaultLocalRulebook,
        schema_version: snapshot.schema_version,
        read_at: snapshot.read_at.to_string(),
        window_start: window.start().to_string(),
        window_end: window.end().to_string(),
        counts: RuleActivityCounts {
            precision: match counts.precision {
                Precision::Exact => RuleActivityPrecision::Exact,
                Precision::LowerBound => RuleActivityPrecision::LowerBound,
            },
            snapshot: RuleActivitySnapshotCounts {
                lines: count(counts.lines)?,
                blank_lines: count(counts.blank_lines)?,
                valid_rows: count(counts.valid_rows)?,
                distinct_ids: count(counts.distinct)?,
                duplicate_rows: count(counts.duplicate_rows)?,
                conflicted_ids: count(counts.conflicted_ids)?,
                conflicted_rows: count(counts.conflicted_rows)?,
                malformed: RuleActivityMalformed {
                    oversize_line: count(malformed.oversize_line)?,
                    invalid_json: count(malformed.invalid_json)?,
                    invalid_shape: count(malformed.invalid_shape)?,
                    invalid_timestamp: count(malformed.invalid_timestamp)?,
                    oversize_value: count(malformed.oversize_value)?,
                },
            },
            window_modes: modes(&counts.in_window)?,
        },
        coverage: RuleActivityCoverage {
            captured_len: coverage.captured_len.to_string(),
            scanned_start: coverage.scanned.start.to_string(),
            scanned_end: coverage.scanned.end.to_string(),
            byte_bound_reached: coverage.byte_bound_reached,
            line_bound_reached: coverage.line_bound_reached,
            leading_partial_dropped: coverage.leading_partial_dropped,
            trailing_partial_dropped: coverage.trailing_partial_dropped,
            grew_after_capture: coverage.grew_after_capture,
            oldest_observed: coverage.oldest_observed.map(|at| at.to_string()),
            newest_observed: coverage.newest_observed.map(|at| at.to_string()),
        },
        latest_fires: snapshot.rows.iter().map(fire).collect(),
        fires_truncated: coverage.row_bound_reached,
        observed_groups: snapshot
            .groups
            .iter()
            .map(group)
            .collect::<Result<_, _>>()?,
        observed_group_count: count(counts.observed_groups)?,
        groups_truncated: coverage.group_bound_reached,
    })
}

fn modes(modes: &ModeCounts) -> Result<RuleActivityModeCounts, Unrepresentable> {
    Ok(RuleActivityModeCounts {
        advise: count(modes.advise)?,
        gate: count(modes.gate)?,
        suppressed: count(modes.suppressed)?,
        unrecognized: count(modes.unrecognized)?,
    })
}

/// The narrowed row: the recorded agent, worktree, repository and branch
/// stay native-only.
fn fire(record: &FireRecord) -> RuleActivityFire {
    RuleActivityFire {
        fire_id: record.fire_id.clone(),
        rule_id: record.rule_id.clone(),
        rule_version: record.rule_version.as_ref().map(|version| match version {
            RuleVersion::Number(number) => RuleActivityVersion::Number {
                value: number.to_string(),
            },
            RuleVersion::Label(label) => RuleActivityVersion::Label {
                value: label.clone(),
            },
        }),
        rulebook_id: record.rulebook_id.clone(),
        fired_at: record.fired_at.to_string(),
        tool: record.tool.clone(),
        hook_phase: record.hook_phase.clone(),
        host: record.host.clone(),
        session_id: record.session_id.clone(),
        mode: match &record.mode {
            FireMode::Advise => RuleActivityMode::Advise,
            FireMode::Gate => RuleActivityMode::Gate,
            FireMode::Suppressed => RuleActivityMode::Suppressed,
            FireMode::Unrecognized(value) => RuleActivityMode::Unrecognized {
                value: value.clone(),
            },
        },
    }
}

fn group(group: &ObservedGroup) -> Result<RuleActivityGroup, Unrepresentable> {
    Ok(RuleActivityGroup {
        rulebook_id: group.rulebook_id.clone(),
        rule_id: group.rule_id.clone(),
        latest_observed: group.latest_observed.to_string(),
        modes: modes(&group.modes)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_past_the_exact_javascript_range_are_refused() {
        assert_eq!(count(MAX_SAFE_INTEGER), Ok(MAX_SAFE_INTEGER));
        for value in [MAX_SAFE_INTEGER + 1, u64::MAX] {
            assert_eq!(count(value), Err(Unrepresentable));
        }
        let over = ModeCounts {
            unrecognized: MAX_SAFE_INTEGER + 1,
            ..ModeCounts::default()
        };
        assert_eq!(modes(&over), Err(Unrepresentable));
    }

    fn window() -> ActivityWindow {
        ActivityWindow::new(
            "2026-09-09T00:00:00Z".parse().unwrap(),
            "2026-09-23T00:00:00Z".parse().unwrap(),
        )
        .unwrap()
    }

    /// Every non-snapshot outcome of the reader maps to a fixed safe shape:
    /// no path, no I/O detail, no counts.
    #[test]
    fn every_stopped_read_maps_to_a_fixed_shape() {
        use serde_json::json;
        let cases = [
            (
                ActivityRead::Unavailable(Unavailable::RootNotAbsolute),
                json!({"state":"unavailable","source":"default_local_rulebook","part":"root","reason":"unsafe_path","found_version":null}),
            ),
            (
                ActivityRead::Unavailable(Unavailable::Path {
                    part: SourcePart::LedgerDir,
                    problem: PathProblem::Missing,
                }),
                json!({"state":"unavailable","source":"default_local_rulebook","part":"ledger_dir","reason":"missing","found_version":null}),
            ),
            (
                ActivityRead::Unavailable(Unavailable::Path {
                    part: SourcePart::Ledger,
                    problem: PathProblem::Unreadable(std::io::ErrorKind::PermissionDenied),
                }),
                json!({"state":"unavailable","source":"default_local_rulebook","part":"ledger","reason":"unreadable","found_version":null}),
            ),
            (
                ActivityRead::Unavailable(Unavailable::Path {
                    part: SourcePart::Ledger,
                    problem: PathProblem::Changed,
                }),
                json!({"state":"unavailable","source":"default_local_rulebook","part":"ledger","reason":"unreadable","found_version":null}),
            ),
            (
                ActivityRead::Unavailable(Unavailable::SchemaMalformed),
                json!({"state":"unavailable","source":"default_local_rulebook","part":"schema_marker","reason":"malformed_schema","found_version":null}),
            ),
            (
                ActivityRead::Unavailable(Unavailable::SchemaUnsupported { found: u64::MAX }),
                json!({"state":"unavailable","source":"default_local_rulebook","part":"schema_marker","reason":"unsupported_schema","found_version":"18446744073709551615"}),
            ),
            (
                ActivityRead::SourceChanged(SourceChange::Shrunk),
                json!({"state":"source_changed","source":"default_local_rulebook","change":"shrunk"}),
            ),
            (
                ActivityRead::SourceChanged(SourceChange::Replaced),
                json!({"state":"source_changed","source":"default_local_rulebook","change":"replaced"}),
            ),
            (
                ActivityRead::SourceChanged(SourceChange::Removed),
                json!({"state":"source_changed","source":"default_local_rulebook","change":"removed"}),
            ),
            (
                ActivityRead::Interrupted(Interruption::Cancelled),
                json!({"state":"interrupted","reason":"cancelled"}),
            ),
            (
                ActivityRead::Interrupted(Interruption::DeadlineExceeded),
                json!({"state":"interrupted","reason":"deadline"}),
            ),
        ];
        for (read, expected) in cases {
            let result = RuleActivityResult::from_read("read-1", window(), read);
            assert_eq!(serde_json::to_value(&result).unwrap(), expected);
        }
        for problem in [
            PathProblem::Symlink,
            PathProblem::NotDirectory,
            PathProblem::NotRegularFile,
            PathProblem::MultipleLinks,
            PathProblem::OutsideRoot,
        ] {
            assert_eq!(
                RuleActivityUnavailableReason::from(problem),
                RuleActivityUnavailableReason::UnsafePath
            );
        }
        let fixed = [
            (
                RuleActivityResult::not_configured(),
                json!({"state":"unavailable","source":"default_local_rulebook","part":"root","reason":"not_configured","found_version":null}),
            ),
            (
                RuleActivityResult::Busy {
                    reason: RuleActivityBusy::AnotherRead,
                },
                json!({"state":"busy","reason":"another_read"}),
            ),
            (
                RuleActivityResult::Busy {
                    reason: RuleActivityBusy::DuplicateRead,
                },
                json!({"state":"busy","reason":"duplicate_read"}),
            ),
            (RuleActivityResult::Closed, json!({"state":"closed"})),
            (RuleActivityResult::Failed, json!({"state":"failed"})),
            (
                RuleActivityResult::InvalidReadId,
                json!({"state":"invalid_read_id"}),
            ),
        ];
        for (result, expected) in fixed {
            assert_eq!(serde_json::to_value(&result).unwrap(), expected);
        }
    }

    #[test]
    fn a_large_integer_version_is_carried_as_exact_text() {
        let record = FireRecord {
            fire_id: "f".into(),
            rule_id: "r".into(),
            rule_version: Some(RuleVersion::Number(i64::MAX)),
            rulebook_id: None,
            session_id: "s".into(),
            agent_id: Some("agent-secret".into()),
            worktree: Some("worktree-secret".into()),
            host: None,
            repo: Some("repo-secret".into()),
            branch: Some("branch-secret".into()),
            tool: None,
            hook_phase: None,
            mode: FireMode::Unrecognized("shadow".into()),
            fired_at: "2026-09-22T10:00:00.5+02:00".parse().unwrap(),
        };
        let wire = serde_json::to_value(fire(&record)).unwrap();
        assert_eq!(
            wire,
            serde_json::json!({
                "fire_id": "f", "rule_id": "r",
                "rule_version": {"kind": "number", "value": "9223372036854775807"},
                "rulebook_id": null, "fired_at": "2026-09-22T08:00:00.5Z",
                "tool": null, "hook_phase": null, "host": null, "session_id": "s",
                "mode": {"kind": "unrecognized", "value": "shadow"}
            })
        );
    }
}
