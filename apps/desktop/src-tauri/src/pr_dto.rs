//! Serialized pull-request contracts: the stored linked pull requests, and
//! what one manual refresh batch did.
//!
//! These shapes carry canonical GitHub identities, the refresh-owned metadata
//! the storage layer already holds, and typed outcome codes. They never carry
//! an executable path, a command line, an environment value, a credential, a
//! process's standard error, a raw response body or any local filesystem
//! path: a failure is one of the storage layer's closed codes and nothing
//! more.
use serde::{Deserialize, Serialize};
use ts_rs::TS;
use xt_store::pr_link::{
    LinkedPullRequest, PrIdentity, PrRefreshError, PrRefreshStatus, PrState, RefreshWrite,
};

/// Integers cross IPC as JSON numbers; anything outside the exactly
/// representable range fails the command rather than being rounded.
pub const MAX_EXACT: i64 = (1_i64 << 53) - 1;

pub fn exact(value: i64, what: &'static str) -> Result<i64, &'static str> {
    if (0..=MAX_EXACT).contains(&value) {
        Ok(value)
    } else {
        Err(what)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PrStateReport {
    Open,
    Closed,
    Merged,
}

impl From<PrState> for PrStateReport {
    fn from(value: PrState) -> Self {
        match value {
            PrState::Open => Self::Open,
            PrState::Closed => Self::Closed,
            PrState::Merged => Self::Merged,
        }
    }
}

/// The storage layer's closed failure vocabulary, exactly as stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PrRefreshErrorCode {
    Unavailable,
    Timeout,
    Cancelled,
    OutputTooLarge,
    InvalidResponse,
    ExecutionFailed,
    NotFound,
    Unauthorized,
    RateLimited,
}

impl From<PrRefreshError> for PrRefreshErrorCode {
    fn from(value: PrRefreshError) -> Self {
        match value {
            PrRefreshError::Unavailable => Self::Unavailable,
            PrRefreshError::Timeout => Self::Timeout,
            PrRefreshError::Cancelled => Self::Cancelled,
            PrRefreshError::OutputTooLarge => Self::OutputTooLarge,
            PrRefreshError::InvalidResponse => Self::InvalidResponse,
            PrRefreshError::ExecutionFailed => Self::ExecutionFailed,
            PrRefreshError::NotFound => Self::NotFound,
            PrRefreshError::Unauthorized => Self::Unauthorized,
            PrRefreshError::RateLimited => Self::RateLimited,
        }
    }
}

/// Facts derived from the stored refresh columns alone; no age or staleness
/// policy is applied here or in storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PrRefreshStatusReport {
    NeverAttempted,
    Refreshed,
    FailedNeverRefreshed { error: PrRefreshErrorCode },
    FailedAfterRefresh { error: PrRefreshErrorCode },
}

impl From<PrRefreshStatus> for PrRefreshStatusReport {
    fn from(value: PrRefreshStatus) -> Self {
        match value {
            PrRefreshStatus::NeverAttempted => Self::NeverAttempted,
            PrRefreshStatus::Refreshed => Self::Refreshed,
            PrRefreshStatus::FailedNeverRefreshed(error) => Self::FailedNeverRefreshed {
                error: error.into(),
            },
            PrRefreshStatus::FailedAfterRefresh(error) => Self::FailedAfterRefresh {
                error: error.into(),
            },
        }
    }
}

/// One canonical pull request, named by the stored ID the refresh command
/// accepts. The ID is a local storage row identifier, not a GitHub one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct PrRef {
    #[ts(type = "number")]
    pub id: i64,
    /// Canonical lowercase `owner/repo`.
    pub repository: String,
    #[ts(type = "number")]
    pub number: i64,
    /// The single canonical URL for this identity.
    pub url: String,
}

impl PrRef {
    pub fn new(id: i64, identity: &PrIdentity) -> Result<Self, &'static str> {
        Ok(Self {
            id: exact(id, "pull request id exceeds the exact JSON integer range")?,
            repository: identity.repository().to_owned(),
            number: exact(
                i64::try_from(identity.number()).unwrap_or(i64::MAX),
                "pull request number exceeds the exact JSON integer range",
            )?,
            url: identity.url(),
        })
    }
}

/// One stored pull request with its refresh-owned metadata. Every metadata
/// field stays `null` until a refresh has filled it; a link never sets one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct PrRow {
    pub pull_request: PrRef,
    /// Sessions that still link this pull request; always at least one.
    pub linked_sessions: u32,
    pub title: Option<String>,
    pub state: Option<PrStateReport>,
    pub merged_at: Option<String>,
    #[ts(type = "number | null")]
    pub additions: Option<i64>,
    #[ts(type = "number | null")]
    pub deletions: Option<i64>,
    pub head_ref_name: Option<String>,
    /// UTC milliseconds of the last successful refresh.
    #[ts(type = "number | null")]
    pub refreshed_at_ms: Option<i64>,
    /// UTC milliseconds of the newest applied attempt, successful or not.
    #[ts(type = "number | null")]
    pub last_attempted_at_ms: Option<i64>,
    pub status: PrRefreshStatusReport,
}

impl PrRow {
    pub fn new(row: &LinkedPullRequest) -> Result<Self, &'static str> {
        let stored = &row.pull_request;
        let linked_sessions = u32::try_from(row.linked_sessions)
            .map_err(|_| "pull request link count exceeds the exact JSON integer range")?;
        let size = |value: Option<i64>| {
            value
                .map(|value| {
                    exact(
                        value,
                        "pull request size exceeds the exact JSON integer range",
                    )
                })
                .transpose()
        };
        let instant = |value: Option<i64>| {
            value
                .map(|value| {
                    exact(
                        value,
                        "pull request time exceeds the exact JSON integer range",
                    )
                })
                .transpose()
        };
        Ok(Self {
            pull_request: PrRef::new(stored.id, &stored.identity)?,
            linked_sessions,
            title: stored.title.clone(),
            state: stored.state.map(Into::into),
            merged_at: stored.merged_at.clone(),
            additions: size(stored.additions)?,
            deletions: size(stored.deletions)?,
            head_ref_name: stored.head_ref_name.clone(),
            refreshed_at_ms: instant(stored.refreshed_at)?,
            last_attempted_at_ms: instant(stored.last_attempted_at)?,
            status: stored.refresh_status().into(),
        })
    }
}

/// Every stored pull request that a session still links, in repository then
/// number order.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct PrList {
    pub rows: Vec<PrRow>,
}

/// What storage did with one attempt's typed result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PrRefreshWriteReport {
    /// The attempt was newer than the stored one and was committed.
    Applied,
    /// The same result was already stored for this attempt; no write.
    Unchanged,
    /// A newer attempt was already stored; nothing was written.
    Stale,
}

impl From<RefreshWrite> for PrRefreshWriteReport {
    fn from(value: RefreshWrite) -> Self {
        match value {
            RefreshWrite::Applied => Self::Applied,
            RefreshWrite::Unchanged => Self::Unchanged,
            RefreshWrite::Stale => Self::Stale,
        }
    }
}

/// What storage did with one executed attempt's result.
///
/// This is deliberately separate from the attempt's own outcome. An attempt
/// that ran and answered is a success or a failure whether or not storage
/// could keep what it answered, so a rejected or impossible write never turns
/// an execution into a non-event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "persistence", rename_all = "snake_case")]
pub enum PrPersistence {
    /// Storage took the result and decided what to do with it.
    Recorded { write: PrRefreshWriteReport },
    /// The result is not stored: this pull request's stored status is exactly
    /// what it was before the attempt ran.
    NotRecorded { reason: PrPersistenceError },
}

/// Why an executed attempt's result is not stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PrPersistenceError {
    /// Storage closed before the result could be recorded.
    StorageClosed,
    /// Storage's rules refused this one result — for example two different
    /// answers stamped with the same attempt time. The batch goes on.
    Refused,
    /// Storage itself failed (an SQLite I/O, corruption or full-disk error,
    /// for example). Nothing after this is attempted.
    Unusable,
}

/// Why a selected pull request was never attempted. A skipped pull request is
/// not a failed one and not an unrecorded one: nothing ran for it, so its
/// stored status is exactly what it was before the batch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum PrSkipReason {
    /// No stored pull request has this ID.
    NotStored,
    /// Stored, but no session links it any more.
    NotLinked,
    /// The batch was cancelled before this pull request was attempted.
    Cancelled,
    /// The batch's total time budget was spent.
    BudgetExhausted,
    /// Storage was no longer usable when this pull request came up, so no
    /// attempt was made for it.
    StorageUnavailable,
    /// The attempt could not be started at all (for example, a host clock the
    /// storage layer cannot stamp an attempt with).
    AttemptRefused,
}

/// What became of one selected pull request. `succeeded` and `failed` report
/// what the attempt itself did; `persistence` reports, separately, what
/// storage then did with that result.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum PrAttemptOutcome {
    /// The attempt ran and returned a pull request.
    Succeeded { persistence: PrPersistence },
    /// The attempt ran and returned one of the closed failure codes.
    Failed {
        error: PrRefreshErrorCode,
        persistence: PrPersistence,
    },
    /// The attempt never ran.
    Skipped { reason: PrSkipReason },
}

impl PrAttemptOutcome {
    /// Whether the attempt executed, whatever storage then did.
    pub fn attempted(&self) -> bool {
        !matches!(self, Self::Skipped { .. })
    }

    pub fn persistence(&self) -> Option<PrPersistence> {
        match self {
            Self::Succeeded { persistence } | Self::Failed { persistence, .. } => {
                Some(*persistence)
            }
            Self::Skipped { .. } => None,
        }
    }
}

/// One selected pull request's place in the batch, in the order it was
/// selected. `pull_request` is absent only for an ID that named no stored row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct PrRefreshRow {
    #[ts(type = "number")]
    pub id: i64,
    pub pull_request: Option<PrRef>,
    pub outcome: PrAttemptOutcome,
}

/// What one manual refresh batch did.
///
/// `requested` equals `attempted + skipped`, and `attempted` equals
/// `succeeded + failed`, so a selection is always fully accounted for.
/// `attempted` counts executions, not writes: an attempt whose result storage
/// would not take is still attempted, and is counted again in `unrecorded`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct PrRefreshReport {
    pub requested: u32,
    /// Attempts that ran, whatever storage then did with their results.
    pub attempted: u32,
    pub succeeded: u32,
    pub failed: u32,
    /// Selected pull requests nothing ran for.
    pub skipped: u32,
    /// Attempts that ran whose result is not stored; a subset of `attempted`.
    pub unrecorded: u32,
    /// A cancellation was observed while the batch ran.
    pub cancelled: bool,
    /// At least one attempt's result was committed, which is also the only
    /// condition under which the refresh event is emitted.
    pub committed: bool,
    pub rows: Vec<PrRefreshRow>,
}
