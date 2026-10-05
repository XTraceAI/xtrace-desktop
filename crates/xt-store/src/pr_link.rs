//! Local pull-request evidence: one canonical identity per GitHub pull request
//! and one confidence-bearing link per canonical session and pull request.
//!
//! Only `https://github.com/<owner>/<repo>/pull/<number>` identities are
//! accepted; enterprise hosts are not supported yet. GitHub resolves owner and
//! repository names case-insensitively and cannot hold two repositories whose
//! names differ only by ASCII case, so the canonical spelling is the ASCII
//! lowercase `owner/repo` plus the decimal number. Lowercasing therefore never
//! merges two distinct pull requests. Every other spelling difference (a
//! trailing slash, `.git`, percent escapes, leading zeroes, `www.`) is rejected
//! instead of being normalized.
//!
//! A link records that a session referenced a pull request; it never implies
//! that the pull request was merged. The refresh-owned columns (`title`, `state`,
//! `merged_at`, `additions`, `deletions`, `head_ref_name`, `refreshed_at`) and
//! the refresh status (`last_attempted_at`, `refresh_error`) are never written
//! by a link. Scanning native files and deciding which session a copied witness
//! belongs to happen in the ingest writer, which commits exact witnesses through
//! the same link write inside its batch (`crate::batch`).
//!
//! `Store::record_pr_refresh` persists the typed result of one refresh attempt
//! for an identity that already has a row. Fetching from GitHub (a client,
//! subprocess or network call), scheduling and staleness policy belong to later
//! owners. This module only persists identities, links and refresh results that
//! a caller has already observed.

use crate::{Error, Result, Store, model::text_enum};
use rusqlite::{Connection, OptionalExtension, Row, TransactionBehavior, params};
use serde::{Deserialize, Serialize};

const PREFIX: &str = "https://github.com/";
/// Longest accepted URL: the prefix, a 39-byte owner, a 100-byte repository,
/// `/pull/` and a 19-digit number fit well within this bound.
pub const MAX_URL_LEN: usize = 256;
const MAX_OWNER_LEN: usize = 39;
const MAX_REPO_LEN: usize = 100;
/// `i64::MAX` has 19 decimal digits; SQLite stores the number as INTEGER.
const MAX_NUMBER_DIGITS: usize = 19;

text_enum!(PrConfidence { Exact => "exact", Sha => "sha", Inferred => "inferred" });
text_enum!(PrState { Open => "OPEN", Closed => "CLOSED", Merged => "MERGED" });
text_enum!(PrRefreshError {
    Unavailable => "unavailable",
    Timeout => "timeout",
    Cancelled => "cancelled",
    OutputTooLarge => "output_too_large",
    InvalidResponse => "invalid_response",
    ExecutionFailed => "execution_failed",
    NotFound => "not_found",
    Unauthorized => "unauthorized",
    RateLimited => "rate_limited",
});

/// Longest accepted pull-request title, in bytes.
pub const MAX_TITLE_LEN: usize = 1024;
/// Longest accepted head branch name, in bytes (Git's own ref-name limit).
pub const MAX_HEAD_REF_LEN: usize = 255;
/// Longest accepted RFC3339 `merged_at` spelling, in bytes.
pub const MAX_MERGED_AT_LEN: usize = 64;
/// Latest accepted attempt: 9999-12-31T23:59:59.999Z in UTC milliseconds.
pub const MAX_ATTEMPTED_AT: i64 = 253_402_300_799_999;

impl PrConfidence {
    /// Strength order: `exact` > `sha` > `inferred`.
    pub(crate) fn rank(self) -> u8 {
        match self {
            Self::Exact => 2,
            Self::Sha => 1,
            Self::Inferred => 0,
        }
    }

    /// The stronger of two confidences. A link can upgrade but never downgrade.
    pub fn strongest(self, other: Self) -> Self {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

/// A validated canonical pull-request identity. Fields are private so every
/// value has passed the same bounds and case normalization.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrIdentity {
    repository: String,
    number: i64,
}

impl PrIdentity {
    /// Parse one canonical GitHub pull-request URL. Scheme and host compare
    /// ASCII case-insensitively; owner and repository are lowercased.
    pub fn from_url(url: &str) -> Result<Self> {
        bounded(url)?;
        // Anything between the scheme and the first path slash other than the
        // bare host (userinfo, a port, another host) fails this comparison.
        if !url
            .get(..PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(PREFIX))
        {
            return Err(Error::InvalidInput(
                "pull request URL must start with https://github.com/",
            ));
        }
        let segments: Vec<&str> = url[PREFIX.len()..].split('/').collect();
        let [owner, repo, "pull", number] = segments.as_slice() else {
            return Err(Error::InvalidInput(
                "pull request URL must be /<owner>/<repo>/pull/<number>",
            ));
        };
        Ok(Self {
            repository: repository(owner, repo)?,
            number: parse_number(number)?,
        })
    }

    /// Build an identity from an `owner/repo` repository and a positive number.
    pub fn from_parts(repository_name: &str, number: u64) -> Result<Self> {
        Ok(Self {
            repository: parse_repository(repository_name)?,
            number: checked_number(number)?,
        })
    }

    /// Reconcile independently supplied observations of one pull request. At
    /// least the URL or both repository and number are required. Every supplied
    /// value must name the same canonical identity; ASCII case differences in
    /// owner/repository reconcile, any other difference fails.
    pub fn reconcile(
        url: Option<&str>,
        repository_name: Option<&str>,
        number: Option<u64>,
    ) -> Result<Self> {
        let from_url = url.map(Self::from_url).transpose()?;
        let repository = repository_name.map(parse_repository).transpose()?;
        let number = number.map(checked_number).transpose()?;
        let identity = match (from_url, repository, number) {
            (Some(identity), repository, number) => {
                if repository.is_some_and(|repository| repository != identity.repository)
                    || number.is_some_and(|number| number != identity.number)
                {
                    return Err(Error::InvalidInput(
                        "pull request URL, repository and number disagree",
                    ));
                }
                identity
            }
            (None, Some(repository), Some(number)) => Self { repository, number },
            (None, _, _) => {
                return Err(Error::InvalidInput(
                    "pull request identity needs a URL or a repository and number",
                ));
            }
        };
        Ok(identity)
    }

    /// Lowercase `owner/repo`.
    pub fn repository(&self) -> &str {
        &self.repository
    }

    pub fn number(&self) -> u64 {
        self.number as u64
    }

    /// The single canonical URL for this identity.
    pub fn url(&self) -> String {
        format!("{PREFIX}{}/pull/{}", self.repository, self.number)
    }

    /// Stored rows must already be canonical; anything else is reported rather
    /// than silently re-spelled.
    pub(crate) fn from_stored(repository_name: &str, number: i64, url: &str) -> Result<Self> {
        let identity = Self::from_url(url)?;
        if identity.url() != url
            || identity.repository != repository_name
            || identity.number != number
        {
            return Err(Error::InvalidInput(
                "stored pull request identity is not canonical",
            ));
        }
        Ok(identity)
    }
}

fn bounded(value: &str) -> Result<()> {
    if value.len() > MAX_URL_LEN {
        return Err(Error::InvalidInput("pull request identity is too long"));
    }
    Ok(())
}

fn parse_repository(name: &str) -> Result<String> {
    bounded(name)?;
    let (owner, repo) = name.split_once('/').ok_or(Error::InvalidInput(
        "pull request repository must be <owner>/<repo>",
    ))?;
    repository(owner, repo)
}

fn repository(owner: &str, repo: &str) -> Result<String> {
    // Owner: ASCII letters, digits and hyphens, not leading, as GitHub logins.
    let owner_valid = (1..=MAX_OWNER_LEN).contains(&owner.len())
        && owner
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        && !owner.starts_with('-');
    // Repository: ASCII letters, digits, `.`, `_` and `-`, excluding the dot
    // traversal names and a `.git` suffix, which GitHub treats as an alias of
    // the bare name. Percent escapes, whitespace and controls never pass.
    let repo_valid = (1..=MAX_REPO_LEN).contains(&repo.len())
        && repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        && repo != "."
        && repo != ".."
        && !repo.to_ascii_lowercase().ends_with(".git");
    if !owner_valid || !repo_valid {
        return Err(Error::InvalidInput(
            "pull request owner or repository is malformed",
        ));
    }
    Ok(format!(
        "{}/{}",
        owner.to_ascii_lowercase(),
        repo.to_ascii_lowercase()
    ))
}

fn parse_number(text: &str) -> Result<i64> {
    let canonical = (1..=MAX_NUMBER_DIGITS).contains(&text.len())
        && !text.starts_with('0')
        && text.bytes().all(|b| b.is_ascii_digit());
    if !canonical {
        return Err(Error::InvalidInput(
            "pull request number must be a canonical positive integer",
        ));
    }
    text.parse::<i64>()
        .map_err(|_| Error::InvalidInput("pull request number is too large"))
}

fn checked_number(number: u64) -> Result<i64> {
    i64::try_from(number)
        .ok()
        .filter(|number| *number > 0)
        .ok_or(Error::InvalidInput(
            "pull request number must be a positive integer within range",
        ))
}

/// One observation that a canonical session referenced a pull request. The
/// shape has no title, branch, path, command, tool argument or transcript text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrLinkObservation {
    pub session_id: String,
    pub pull_request: PrIdentity,
    pub confidence: PrConfidence,
    /// Caller-supplied UTC milliseconds; `first_seen_at <= last_seen_at`.
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

/// The stored row for a canonical pull request. The refresh-owned fields stay
/// unknown until a successful refresh fills them; a link never sets them.
/// `refreshed_at` is the last successful refresh, `last_attempted_at` the newest
/// applied attempt and `refresh_error` that attempt's typed failure, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredPullRequest {
    pub id: i64,
    pub identity: PrIdentity,
    pub title: Option<String>,
    pub state: Option<PrState>,
    pub merged_at: Option<String>,
    pub additions: Option<i64>,
    pub deletions: Option<i64>,
    pub head_ref_name: Option<String>,
    pub refreshed_at: Option<i64>,
    pub last_attempted_at: Option<i64>,
    pub refresh_error: Option<PrRefreshError>,
}

/// One stored pull request with the number of sessions that still link it,
/// read together so the count and the metadata are from the same instant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedPullRequest {
    pub pull_request: StoredPullRequest,
    pub linked_sessions: i64,
}

/// Facts derived from the stored refresh columns alone. No age or staleness
/// policy is applied; callers compare `refreshed_at`/`last_attempted_at` with
/// their own clock if they need one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrRefreshStatus {
    /// No refresh attempt has been recorded.
    NeverAttempted,
    /// The newest attempt succeeded; the metadata is from that attempt.
    Refreshed,
    /// The newest attempt failed and no refresh ever succeeded.
    FailedNeverRefreshed(PrRefreshError),
    /// The newest attempt failed; the metadata is from the earlier success at
    /// `refreshed_at`.
    FailedAfterRefresh(PrRefreshError),
}

impl StoredPullRequest {
    pub fn refresh_status(&self) -> PrRefreshStatus {
        match (
            self.refresh_error,
            self.refreshed_at,
            self.last_attempted_at,
        ) {
            (Some(error), None, _) => PrRefreshStatus::FailedNeverRefreshed(error),
            (Some(error), Some(_), _) => PrRefreshStatus::FailedAfterRefresh(error),
            (None, None, None) => PrRefreshStatus::NeverAttempted,
            (None, _, _) => PrRefreshStatus::Refreshed,
        }
    }
}

/// One successful refresh of an existing canonical pull request. The caller
/// reconciles the response's URL/repository/number to `pull_request` (for
/// example through `PrIdentity::reconcile`) before building this value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshSuccess {
    pub pull_request: PrIdentity,
    /// Caller-supplied UTC milliseconds, `0..=MAX_ATTEMPTED_AT`.
    pub attempted_at: i64,
    /// Nonblank, control-free, at most `MAX_TITLE_LEN` bytes.
    pub title: String,
    pub state: PrState,
    /// RFC3339, required for `MERGED` and absent for `OPEN`/`CLOSED`.
    pub merged_at: Option<String>,
    pub additions: i64,
    pub deletions: i64,
    /// Nonblank, control-free, at most `MAX_HEAD_REF_LEN` bytes.
    pub head_ref_name: String,
}

/// One failed refresh attempt, as a closed code only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefreshFailure {
    pub pull_request: PrIdentity,
    /// Caller-supplied UTC milliseconds, `0..=MAX_ATTEMPTED_AT`.
    pub attempted_at: i64,
    pub error: PrRefreshError,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RefreshOutcome {
    Success(RefreshSuccess),
    Failure(RefreshFailure),
}

impl RefreshOutcome {
    pub fn pull_request(&self) -> &PrIdentity {
        match self {
            Self::Success(success) => &success.pull_request,
            Self::Failure(failure) => &failure.pull_request,
        }
    }

    pub fn attempted_at(&self) -> i64 {
        match self {
            Self::Success(success) => success.attempted_at,
            Self::Failure(failure) => failure.attempted_at,
        }
    }

    /// The field rules `record_pr_refresh` applies before it opens its
    /// transaction, exposed so a client that builds a result from a response
    /// can reject it by the same rules instead of restating them. It reads no
    /// database state, so it decides nothing about identity, ordering or
    /// conflicts; only the shape of this result.
    pub fn validate(&self) -> Result<()> {
        validate_refresh(self)
    }
}

/// What one `record_pr_refresh` call did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshWrite {
    /// The attempt was newer than the stored one and was committed.
    Applied,
    /// The exact same result was already stored for this attempt; no write.
    Unchanged,
    /// The attempt is older than the stored one; nothing was written.
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredPrLink {
    pub session_id: String,
    pub pull_request: PrIdentity,
    pub confidence: PrConfidence,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

/// What one committed observation changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrLinkOutcome {
    pub pull_request_id: i64,
    pub pull_request_created: bool,
    pub link_created: bool,
    /// False for an idempotent replay that changed nothing.
    pub link_changed: bool,
}

impl Store {
    /// Persist one pull-request stub and its session link in one immediate
    /// transaction. The session must already exist. A pre-existing row whose
    /// URL, repository or number disagrees with the canonical identity fails,
    /// as does any other error; nothing from the call is then committed.
    /// Replays keep the earliest first-seen time, the latest last-seen time and
    /// the strongest confidence, so duplicate and reversed arrivals converge.
    pub fn record_pr_link(&mut self, observation: &PrLinkObservation) -> Result<PrLinkOutcome> {
        validate(observation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let outcome = record_pr_link(&transaction, observation)?;
        transaction.commit()?;
        Ok(outcome)
    }

    /// The stub for one canonical identity. No candidate row returns `None`;
    /// a case variant, URL alias, split identity or several candidates is a
    /// conflict error, exactly as the writer would report it. Stored rows are
    /// never re-spelled.
    pub fn pull_request(&self, identity: &PrIdentity) -> Result<Option<StoredPullRequest>> {
        let transaction = self.connection.unchecked_transaction()?;
        let stored = canonical_row(&transaction, identity)?
            .map(|id| {
                transaction.query_row(
                    &format!("SELECT {PULL_REQUEST_COLUMNS} FROM pull_requests WHERE id=?1"),
                    [id],
                    stored_pull_request,
                )
            })
            .transpose()?
            .transpose()?;
        transaction.commit()?;
        Ok(stored)
    }

    /// Every stored pull request, ordered by repository then number. A
    /// non-canonical stored identity is reported rather than re-spelled.
    pub fn all_pull_requests(&self) -> Result<Vec<StoredPullRequest>> {
        self.connection
            .prepare(&format!(
                "SELECT {PULL_REQUEST_COLUMNS} FROM pull_requests ORDER BY repo, number, id"
            ))?
            .query_map([], stored_pull_request)?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect()
    }

    /// Every stored pull request with the number of sessions that still link
    /// it, in repository then number order.
    ///
    /// Both facts come from one statement in one read transaction, so a writer
    /// on another connection — the native index has its own — cannot be
    /// observed half applied: a row's metadata and its link count are always
    /// from the same instant. Reading the two tables separately could pair a
    /// row read before a commit with a count read after it.
    pub fn linked_pull_requests(&self) -> Result<Vec<LinkedPullRequest>> {
        let transaction = self.connection.unchecked_transaction()?;
        let rows = transaction
            .prepare(&format!(
                "SELECT {PULL_REQUEST_COLUMNS},
                    (SELECT COUNT(*) FROM pr_links WHERE pr_links.pr_id = pull_requests.id)
                 FROM pull_requests ORDER BY repo, number, id"
            ))?
            .query_map([], |row| {
                let stored = stored_pull_request(row)?;
                let linked_sessions = row.get(PULL_REQUEST_COLUMN_COUNT)?;
                Ok(stored.map(|pull_request| LinkedPullRequest {
                    pull_request,
                    linked_sessions,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .collect::<Result<Vec<_>>>()?;
        transaction.commit()?;
        Ok(rows)
    }

    /// Persist one refresh attempt for an existing canonical pull request in one
    /// immediate transaction. The whole result is validated first; an unknown
    /// identity or a conflicting legacy row fails without writing. Links,
    /// confidence and first/last-seen times are never touched.
    ///
    /// Attempts are ordered by `attempted_at`. A newer success replaces every
    /// refresh-owned field, sets `refreshed_at` and `last_attempted_at` to the
    /// attempt and clears the error. A newer failure sets only
    /// `last_attempted_at` and the error, keeping the last successful metadata
    /// and `refreshed_at`. An older attempt returns `Stale` without writing. At
    /// an equal attempt time, an exactly identical result is `Unchanged`; any
    /// other success or failure is a conflict error and nothing is written.
    pub fn record_pr_refresh(&mut self, outcome: &RefreshOutcome) -> Result<RefreshWrite> {
        outcome.validate()?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let write = record_pr_refresh(&transaction, outcome)?;
        transaction.commit()?;
        Ok(write)
    }

    /// Links of one session, ordered by repository then number.
    pub fn session_pr_links(&self, session_id: &str) -> Result<Vec<StoredPrLink>> {
        links(
            &self.connection,
            "l.session_id=?1 ORDER BY p.repo, p.number",
            params![session_id],
        )
    }

    /// Sessions linked to one pull request, ordered by session ID. Candidate
    /// resolution and conflicts match `pull_request`; no candidate is empty.
    pub fn pull_request_links(&self, identity: &PrIdentity) -> Result<Vec<StoredPrLink>> {
        let transaction = self.connection.unchecked_transaction()?;
        let links = match canonical_row(&transaction, identity)? {
            Some(id) => links(&transaction, "p.id=?1 ORDER BY l.session_id", [id])?,
            None => Vec::new(),
        };
        transaction.commit()?;
        Ok(links)
    }

    /// Every link, ordered by repository, number and session ID.
    pub fn all_pr_links(&self) -> Result<Vec<StoredPrLink>> {
        links(
            &self.connection,
            "1 ORDER BY p.repo, p.number, l.session_id",
            [],
        )
    }
}

fn validate(observation: &PrLinkObservation) -> Result<()> {
    if observation.session_id.trim().is_empty() {
        return Err(Error::InvalidInput("pull request link session is empty"));
    }
    if observation.first_seen_at > observation.last_seen_at {
        return Err(Error::InvalidInput(
            "pull request link first_seen_at is after last_seen_at",
        ));
    }
    Ok(())
}

/// The link write shared by `Store::record_pr_link` and the ingest batch, run
/// under the caller's immediate transaction so a failure rolls back whatever
/// else that transaction wrote.
pub(crate) fn record_pr_link(
    connection: &Connection,
    observation: &PrLinkObservation,
) -> Result<PrLinkOutcome> {
    validate(observation)?;
    let exists = connection
        .query_row(
            "SELECT 1 FROM sessions WHERE session_id=?1",
            [&observation.session_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Err(Error::InvalidInput(
            "session must be created before linking a pull request",
        ));
    }
    let identity = &observation.pull_request;
    let (pull_request_id, pull_request_created) = match canonical_row(connection, identity)? {
        Some(id) => (id, false),
        None => {
            connection.execute(
                "INSERT INTO pull_requests(repo,number,url) VALUES (?1,?2,?3)",
                params![identity.repository, identity.number, identity.url()],
            )?;
            (connection.last_insert_rowid(), true)
        }
    };
    let existing = connection
        .query_row(
            "SELECT confidence,first_seen_at,last_seen_at FROM pr_links WHERE session_id=?1 AND pr_id=?2",
            params![observation.session_id, pull_request_id],
            |row| {
                Ok((
                    row.get::<_, PrConfidence>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .optional()?;
    let merged = match existing {
        None => (
            observation.confidence,
            observation.first_seen_at,
            observation.last_seen_at,
        ),
        Some((confidence, first, last)) => (
            confidence.strongest(observation.confidence),
            first.min(observation.first_seen_at),
            last.max(observation.last_seen_at),
        ),
    };
    let link_changed = existing != Some(merged);
    if link_changed {
        connection.execute(
            "INSERT INTO pr_links(session_id,pr_id,confidence,first_seen_at,last_seen_at)
             VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(session_id,pr_id) DO UPDATE SET confidence=excluded.confidence,
                 first_seen_at=excluded.first_seen_at, last_seen_at=excluded.last_seen_at",
            params![
                observation.session_id,
                pull_request_id,
                merged.0,
                merged.1,
                merged.2
            ],
        )?;
    }
    Ok(PrLinkOutcome {
        pull_request_id,
        pull_request_created,
        link_created: existing.is_none(),
        link_changed,
    })
}

/// Resolve the one stored row for a canonical identity, shared by the writer
/// and the identity readers. Candidates match ASCII case-insensitively by
/// repository/number or by URL, so a differently spelled existing row is found
/// and reported rather than hidden or duplicated. `None` means no candidate; a
/// single exactly canonical candidate is returned; anything else (a case
/// variant, URL alias, split identity or several candidates) is a conflict.
fn canonical_row(connection: &Connection, identity: &PrIdentity) -> Result<Option<i64>> {
    let url = identity.url();
    let mut candidates = connection
        .prepare(
            "SELECT id,repo,number,url FROM pull_requests
             WHERE (number=?2 AND lower(repo)=?1) OR lower(url)=?3",
        )?
        .query_map(params![identity.repository, identity.number, url], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    match candidates.pop() {
        None => Ok(None),
        Some((id, repo, number, stored_url))
            if candidates.is_empty()
                && PrIdentity::from_stored(&repo, number, &stored_url)
                    .is_ok_and(|stored| stored == *identity) =>
        {
            Ok(Some(id))
        }
        Some(_) => Err(Error::InvalidInput(
            "an existing pull request row conflicts with this identity",
        )),
    }
}

const PULL_REQUEST_COLUMNS: &str = "id,repo,number,url,title,state,merged_at,additions,deletions,\
     head_ref_name,refreshed_at,last_attempted_at,refresh_error";
/// How many columns `PULL_REQUEST_COLUMNS` selects, so a query that appends
/// one names its index rather than counting the string again.
const PULL_REQUEST_COLUMN_COUNT: usize = 13;

fn stored_pull_request(row: &Row<'_>) -> rusqlite::Result<Result<StoredPullRequest>> {
    let identity = PrIdentity::from_stored(
        &row.get::<_, String>(1)?,
        row.get(2)?,
        &row.get::<_, String>(3)?,
    );
    let id = row.get(0)?;
    let refresh = refresh_columns(row, 4)?;
    let (last_attempted_at, refresh_error) = (row.get(11)?, row.get(12)?);
    Ok(identity.map(|identity| StoredPullRequest {
        id,
        identity,
        title: refresh.title,
        state: refresh.state,
        merged_at: refresh.merged_at,
        additions: refresh.additions,
        deletions: refresh.deletions,
        head_ref_name: refresh.head_ref_name,
        refreshed_at: refresh.refreshed_at,
        last_attempted_at,
        refresh_error,
    }))
}

/// The refresh-owned columns in stored order, compared whole on an equal
/// attempt so a replay is either byte-identical or a conflict.
#[derive(Debug, PartialEq, Eq)]
struct RefreshColumns {
    title: Option<String>,
    state: Option<PrState>,
    merged_at: Option<String>,
    additions: Option<i64>,
    deletions: Option<i64>,
    head_ref_name: Option<String>,
    refreshed_at: Option<i64>,
}

fn refresh_columns(row: &Row<'_>, start: usize) -> rusqlite::Result<RefreshColumns> {
    Ok(RefreshColumns {
        title: row.get(start)?,
        state: row.get(start + 1)?,
        merged_at: row.get(start + 2)?,
        additions: row.get(start + 3)?,
        deletions: row.get(start + 4)?,
        head_ref_name: row.get(start + 5)?,
        refreshed_at: row.get(start + 6)?,
    })
}

fn validate_refresh(outcome: &RefreshOutcome) -> Result<()> {
    if !(0..=MAX_ATTEMPTED_AT).contains(&outcome.attempted_at()) {
        return Err(Error::InvalidInput(
            "pull request refresh attempt time is out of range",
        ));
    }
    let RefreshOutcome::Success(success) = outcome else {
        return Ok(());
    };
    refresh_text(&success.title, MAX_TITLE_LEN)?;
    refresh_text(&success.head_ref_name, MAX_HEAD_REF_LEN)?;
    if success.additions < 0 || success.deletions < 0 {
        return Err(Error::InvalidInput(
            "pull request additions and deletions must be nonnegative",
        ));
    }
    match (success.state, &success.merged_at) {
        (PrState::Merged, Some(merged_at)) => {
            if merged_at.len() > MAX_MERGED_AT_LEN || crate::timestamp::parse(merged_at).is_err() {
                return Err(Error::InvalidInput(
                    "pull request merged_at must be a bounded RFC3339 timestamp",
                ));
            }
        }
        (PrState::Merged, None) => {
            return Err(Error::InvalidInput(
                "a merged pull request requires merged_at",
            ));
        }
        (PrState::Open | PrState::Closed, Some(_)) => {
            return Err(Error::InvalidInput(
                "an open or closed pull request cannot have merged_at",
            ));
        }
        (PrState::Open | PrState::Closed, None) => {}
    }
    Ok(())
}

/// Title and head branch: bounded, nonblank and free of control characters.
fn refresh_text(value: &str, limit: usize) -> Result<()> {
    if value.len() > limit {
        return Err(Error::InvalidInput("pull request refresh text is too long"));
    }
    if value.trim().is_empty() || value.chars().any(char::is_control) {
        return Err(Error::InvalidInput(
            "pull request refresh text is blank or has control characters",
        ));
    }
    Ok(())
}

fn record_pr_refresh(connection: &Connection, outcome: &RefreshOutcome) -> Result<RefreshWrite> {
    let id = canonical_row(connection, outcome.pull_request())?.ok_or(Error::InvalidInput(
        "pull request must be recorded before it is refreshed",
    ))?;
    let (stored, last_attempted_at, refresh_error) = connection.query_row(
        "SELECT title,state,merged_at,additions,deletions,head_ref_name,refreshed_at,
                last_attempted_at,refresh_error
         FROM pull_requests WHERE id=?1",
        [id],
        |row| {
            Ok((
                refresh_columns(row, 0)?,
                row.get::<_, Option<i64>>(7)?,
                row.get::<_, Option<PrRefreshError>>(8)?,
            ))
        },
    )?;
    let attempted_at = outcome.attempted_at();
    // A success recorded before migration 8 has no attempt time of its own.
    match last_attempted_at.or(stored.refreshed_at) {
        Some(previous) if attempted_at < previous => return Ok(RefreshWrite::Stale),
        Some(previous) if attempted_at == previous => {
            let identical = match outcome {
                RefreshOutcome::Success(success) => {
                    refresh_error.is_none() && stored == success_columns(success)
                }
                RefreshOutcome::Failure(failure) => {
                    last_attempted_at == Some(attempted_at)
                        && refresh_error == Some(failure.error)
                        && stored.refreshed_at != Some(attempted_at)
                }
            };
            return if identical {
                Ok(RefreshWrite::Unchanged)
            } else {
                Err(Error::InvalidInput(
                    "a different refresh result is already stored for this attempt",
                ))
            };
        }
        _ => {}
    }
    match outcome {
        RefreshOutcome::Success(success) => {
            let columns = success_columns(success);
            connection.execute(
                "UPDATE pull_requests SET title=?2,state=?3,merged_at=?4,additions=?5,
                     deletions=?6,head_ref_name=?7,refreshed_at=?8,last_attempted_at=?8,
                     refresh_error=NULL
                 WHERE id=?1",
                params![
                    id,
                    columns.title,
                    columns.state,
                    columns.merged_at,
                    columns.additions,
                    columns.deletions,
                    columns.head_ref_name,
                    attempted_at
                ],
            )?;
        }
        RefreshOutcome::Failure(failure) => {
            connection.execute(
                "UPDATE pull_requests SET last_attempted_at=?2,refresh_error=?3 WHERE id=?1",
                params![id, attempted_at, failure.error],
            )?;
        }
    }
    Ok(RefreshWrite::Applied)
}

fn success_columns(success: &RefreshSuccess) -> RefreshColumns {
    RefreshColumns {
        title: Some(success.title.clone()),
        state: Some(success.state),
        merged_at: success.merged_at.clone(),
        additions: Some(success.additions),
        deletions: Some(success.deletions),
        head_ref_name: Some(success.head_ref_name.clone()),
        refreshed_at: Some(success.attempted_at),
    }
}

fn links(
    connection: &Connection,
    filter: &str,
    parameters: impl rusqlite::Params,
) -> Result<Vec<StoredPrLink>> {
    let sql = format!(
        "SELECT l.session_id,p.repo,p.number,p.url,l.confidence,l.first_seen_at,l.last_seen_at
         FROM pr_links l JOIN pull_requests p ON p.id=l.pr_id WHERE {filter}"
    );
    let rows = connection
        .prepare(&sql)?
        .query_map(parameters, |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, PrConfidence>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(session_id, repo, number, url, confidence, first, last)| {
            Ok(StoredPrLink {
                session_id,
                pull_request: PrIdentity::from_stored(&repo, number, &url)?,
                confidence,
                first_seen_at: first,
                last_seen_at: last,
            })
        })
        .collect()
}
