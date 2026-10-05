//! Typed metadata-only count overlay and separate cancellation registry.
use serde::Serialize;
use ts_rs::TS;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CompactionReason {
    NotIndexed,
    Unsupported,
    Missing,
    Unreadable,
    Ambiguous,
    Replaced,
    IdentityMismatch,
    Ownership,
    Incomplete,
    Limit,
    Cancelled,
}
/// How a recorded compaction started. Codex records no such marker, so its
/// compactions are always `unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CompactionTrigger {
    Auto,
    Manual,
    Unknown,
}
/// One counted compaction's recorded time (UTC milliseconds) and trigger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
pub struct CompactionEvent {
    #[ts(type = "number")]
    pub at_ms: i64,
    pub trigger: CompactionTrigger,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CompactionCount {
    /// `events` holds each counted compaction that recorded a time, oldest
    /// first: Claude Code and Codex markers do; Cursor summaries do not, so a
    /// Cursor count has none. It never changes `count`.
    Count {
        count: u32,
        events: Vec<CompactionEvent>,
    },
    Unknown {
        reason: CompactionReason,
    },
}
impl From<xt_ingest::native::session_compactions::Counted> for CompactionCount {
    fn from(value: xt_ingest::native::session_compactions::Counted) -> Self {
        use xt_ingest::native::session_compactions::Trigger;
        match Self::from(value.outcome) {
            Self::Count { count, .. } => Self::Count {
                count,
                events: value
                    .events
                    .into_iter()
                    .map(|event| CompactionEvent {
                        at_ms: event.at_ms,
                        trigger: match event.trigger {
                            Trigger::Auto => CompactionTrigger::Auto,
                            Trigger::Manual => CompactionTrigger::Manual,
                            Trigger::Unknown => CompactionTrigger::Unknown,
                        },
                    })
                    .collect(),
            },
            unknown => unknown,
        }
    }
}
impl From<xt_ingest::native::session_compactions::Outcome> for CompactionCount {
    fn from(value: xt_ingest::native::session_compactions::Outcome) -> Self {
        use xt_ingest::native::session_compactions::{Outcome, Reason};
        match value {
            Outcome::Count { count } => Self::Count {
                count,
                events: Vec::new(),
            },
            Outcome::Unknown { reason } => Self::Unknown {
                reason: match reason {
                    Reason::NotIndexed => CompactionReason::NotIndexed,
                    Reason::Unsupported => CompactionReason::Unsupported,
                    Reason::Missing => CompactionReason::Missing,
                    Reason::Unreadable => CompactionReason::Unreadable,
                    Reason::Ambiguous => CompactionReason::Ambiguous,
                    Reason::Replaced => CompactionReason::Replaced,
                    Reason::IdentityMismatch => CompactionReason::IdentityMismatch,
                    Reason::Ownership => CompactionReason::Ownership,
                    Reason::Incomplete => CompactionReason::Incomplete,
                    Reason::Limit => CompactionReason::Limit,
                    Reason::Cancelled => CompactionReason::Cancelled,
                },
            },
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct SessionCompaction {
    pub id: String,
    pub outcome: CompactionCount,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, TS)]
pub struct SessionCompactions {
    pub counts: Vec<SessionCompaction>,
}

#[derive(Default)]
pub struct CompactionReads(crate::transcript_reads::TranscriptReads);
impl std::ops::Deref for CompactionReads {
    type Target = crate::transcript_reads::TranscriptReads;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
