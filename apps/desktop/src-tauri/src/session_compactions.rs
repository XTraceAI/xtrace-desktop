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
    /// An approval reviewer's file holds only part of the reviewed
    /// conversation, with earlier compactions left out, and none of its own.
    Snapshot,
    /// A forked conversation's file holds copied history whose compactions
    /// could not be told apart from its own.
    CopiedHistory,
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
/// What a forked conversation inherits: the compactions recorded in the
/// conversation it was forked from before the fork point (for an older fork
/// whose file holds a copy of that history, the copied ones), or why they
/// could not be counted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum InheritedCompactions {
    /// `copied` is present, and true, only for an older fork whose file holds
    /// a copy of that history; a newer fork only references it.
    Count {
        count: u32,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        copied: Option<bool>,
    },
    Unknown {
        reason: CompactionReason,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CompactionCount {
    /// `events` holds each counted compaction that recorded a time, oldest
    /// first: Claude Code and Codex markers do; Cursor summaries do not, so a
    /// Cursor count has none. It never changes `count`.
    ///
    /// `count` and `events` are the session's own. `inherited` is present
    /// only for a forked conversation: what it inherits, kept apart from its
    /// own count. Absent for every other session.
    Count {
        count: u32,
        events: Vec<CompactionEvent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        inherited: Option<InheritedCompactions>,
    },
    Unknown {
        reason: CompactionReason,
    },
}
impl From<xt_ingest::native::session_compactions::Counted> for CompactionCount {
    fn from(value: xt_ingest::native::session_compactions::Counted) -> Self {
        use xt_ingest::native::session_compactions::Trigger;
        let copied = value.copied.then_some(true);
        match Self::from(value.outcome) {
            Self::Count { count, .. } => Self::Count {
                count,
                inherited: value
                    .inherited
                    .map(|inherited| match Self::from(inherited) {
                        Self::Count { count, .. } => InheritedCompactions::Count { count, copied },
                        Self::Unknown { reason } => InheritedCompactions::Unknown { reason },
                    }),
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
                inherited: None,
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
                    Reason::Snapshot => CompactionReason::Snapshot,
                    Reason::CopiedHistory => CompactionReason::CopiedHistory,
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

#[cfg(test)]
mod tests {
    use super::*;
    use xt_ingest::native::session_compactions::{Counted, Outcome, Reason};

    /// A fork's inherited part rides beside its own count; every other
    /// session's answer is unchanged on the wire.
    #[test]
    fn a_fork_carries_its_inherited_part_and_others_do_not() {
        let counted = |inherited| Counted {
            outcome: Outcome::Count { count: 3 },
            events: Vec::new(),
            inherited,
            copied: false,
        };
        let json = |counted: Counted| serde_json::to_value(CompactionCount::from(counted)).unwrap();
        assert_eq!(
            json(counted(None)),
            serde_json::json!({"state": "count", "count": 3, "events": []})
        );
        assert_eq!(
            json(counted(Some(Outcome::Count { count: 0 }))),
            serde_json::json!({"state": "count", "count": 3, "events": [],
                "inherited": {"state": "count", "count": 0}})
        );
        assert_eq!(
            json(counted(Some(Outcome::unknown(Reason::Missing)))),
            serde_json::json!({"state": "count", "count": 3, "events": [],
                "inherited": {"state": "unknown", "reason": "missing"}})
        );
        assert_eq!(
            json(Counted::unknown(Reason::Ownership)),
            serde_json::json!({"state": "unknown", "reason": "ownership"})
        );
        // An older fork's copied part says it is copied.
        let copied = Counted {
            copied: true,
            ..counted(Some(Outcome::Count { count: 49 }))
        };
        assert_eq!(
            json(copied),
            serde_json::json!({"state": "count", "count": 3, "events": [],
                "inherited": {"state": "count", "count": 49, "copied": true}})
        );
    }

    /// The two specific refusals reach the window by their own names.
    #[test]
    fn a_reviewer_snapshot_and_an_unsplit_copy_keep_their_reasons() {
        let json = |counted: Counted| serde_json::to_value(CompactionCount::from(counted)).unwrap();
        assert_eq!(
            json(Counted::unknown(Reason::Snapshot)),
            serde_json::json!({"state": "unknown", "reason": "snapshot"})
        );
        assert_eq!(
            json(Counted::unknown(Reason::CopiedHistory)),
            serde_json::json!({"state": "unknown", "reason": "copied_history"})
        );
    }
}
