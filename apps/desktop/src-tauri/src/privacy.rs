//! Settings over the store's existing content retention policy (P-01) and its
//! explicit transactional purge (P-02). This adapter adds no semantics: the
//! store owns the mode, the registered content inventory and the rollback.
use crate::state::{AppState, StateError};
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xt_store::retention::{ContentRegistry, PurgeOutcome, RetentionMode};

/// The saved future-write mode; an absent setting reads as metadata-only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ContentRetention {
    MetadataOnly,
    FullContent,
}

impl From<RetentionMode> for ContentRetention {
    fn from(mode: RetentionMode) -> Self {
        match mode {
            RetentionMode::MetadataOnly => Self::MetadataOnly,
            RetentionMode::FullContent => Self::FullContent,
        }
    }
}

impl From<ContentRetention> for RetentionMode {
    fn from(mode: ContentRetention) -> Self {
        match mode {
            ContentRetention::MetadataOnly => Self::MetadataOnly,
            ContentRetention::FullContent => Self::FullContent,
        }
    }
}

/// One registered owner's cleared rows, from a committed purge.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct PurgedContentTable {
    pub table: String,
    #[ts(type = "number")]
    pub rows: u64,
}

/// A committed purge. `invalidate_content` is false when no stored field held content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct ContentPurge {
    pub tables: Vec<PurgedContentTable>,
    pub invalidate_content: bool,
}

impl TryFrom<PurgeOutcome> for ContentPurge {
    type Error = StateError;
    fn try_from(outcome: PurgeOutcome) -> Result<Self, StateError> {
        let tables = outcome
            .tables
            .into_iter()
            .map(|table| {
                let rows = u64::try_from(table.rows)
                    .ok()
                    .filter(|rows| *rows < 1_u64 << 53)
                    .ok_or(StateError::CountRange)?;
                Ok(PurgedContentTable {
                    table: table.table,
                    rows,
                })
            })
            .collect::<Result<_, StateError>>()?;
        Ok(Self {
            tables,
            invalidate_content: outcome.invalidate_content,
        })
    }
}

impl AppState {
    pub fn content_retention(&self) -> Result<ContentRetention, StateError> {
        self.with_store(|store| Ok(store.retention_mode()?.into()))
    }

    /// Changes future imports and enrichment only; saved content is untouched.
    /// Returns the mode read back after the write committed.
    pub fn set_content_retention(
        &self,
        mode: ContentRetention,
    ) -> Result<ContentRetention, StateError> {
        self.with_store(|store| {
            store.set_retention_mode(mode.into())?;
            Ok(store.retention_mode()?.into())
        })
    }

    /// The caller owns user confirmation. `invalidate` runs only for a committed
    /// purge that cleared content; a failed or empty purge publishes nothing.
    /// Original host files are never opened.
    pub fn purge_stored_content(
        &self,
        invalidate: impl FnOnce(),
    ) -> Result<ContentPurge, StateError> {
        let outcome = self.with_store(|store| {
            store
                .purge_content(&ContentRegistry::default())
                .map_err(StateError::from)
        });
        publish_committed(outcome, invalidate)
    }
}

fn publish_committed(
    outcome: Result<PurgeOutcome, StateError>,
    invalidate: impl FnOnce(),
) -> Result<ContentPurge, StateError> {
    let outcome = outcome?;
    // The transaction has committed: content-dependent views are stale even if
    // the reply below cannot be represented.
    if outcome.invalidate_content {
        invalidate();
    }
    outcome.try_into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use xt_store::retention::PurgedTable;

    #[test]
    fn failed_purge_publishes_no_invalidation() {
        let mut published = false;
        let result = publish_committed(Err(StateError::Poisoned), || published = true);
        assert!(matches!(result, Err(StateError::Poisoned)));
        assert!(!published);
    }

    #[test]
    fn committed_purge_publishes_only_when_content_was_cleared() {
        for (rows, expected) in [(0, false), (3, true)] {
            let mut published = false;
            let result = publish_committed(
                Ok(PurgeOutcome {
                    tables: vec![PurgedTable {
                        table: "records".into(),
                        rows,
                    }],
                    invalidate_content: rows > 0,
                }),
                || published = true,
            )
            .unwrap();
            assert_eq!(published, expected);
            assert_eq!(result.invalidate_content, expected);
            assert_eq!(result.tables[0].rows, rows as u64);
        }
    }

    #[test]
    fn retention_wire_names_match_the_store() {
        for mode in [RetentionMode::MetadataOnly, RetentionMode::FullContent] {
            assert_eq!(
                serde_json::to_value(ContentRetention::from(mode)).unwrap(),
                serde_json::to_value(mode).unwrap()
            );
            assert_eq!(RetentionMode::from(ContentRetention::from(mode)), mode);
        }
    }
}
