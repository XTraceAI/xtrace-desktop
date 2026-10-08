//! Content-free, bounded session metadata for the initial Sessions browser.
use crate::{Error, Result, Store};
use serde::{Deserialize, Serialize};

/// The indexed timestamp used to order the whole list before paging.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionSort {
    #[default]
    Started,
    RecentlyActive,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCursor {
    pub time: i64,
    #[serde(default)]
    pub sort: SessionSort,
    pub id: String,
}
#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub id: String,
    pub host: String,
    /// The title already saved for this session, when a source supplied one.
    /// Blank titles read as absent. Nothing here derives a title from content.
    pub title: Option<String>,
    /// The session start shown on its row. For Codex and Cursor this is only
    /// the start the host recorded; a missing one stays unknown. Claude Code
    /// records no start, so a Claude session without a stored one starts at
    /// its own first event: its earliest work record, the event M-16's
    /// sessions per day counts it by. Records a fork shares with the session
    /// it was forked from are that session's work and never set a fork's
    /// start: a fork with no work of its own has no shown start and is ordered
    /// by its first recorded time instead. The stored
    /// `sessions.started_at_ms` is never changed.
    pub started_at_ms: Option<i64>,
    pub repo: Option<String>,
    pub branch: Option<String>,
    /// The model the session used most over its whole history
    /// ([`crate::session_model`]), or `None` when nothing named one.
    pub model: Option<String>,
    /// How many other models the same work named besides `model`.
    pub other_models: u64,
    /// The earliest visible work record, counting copies inherited from a
    /// forked session; not a session start.
    pub first_ts: Option<String>,
    /// Latest timed own work, excluding metadata and inherited fork copies.
    pub last_activity_at_ms: Option<i64>,
    pub record_count: u64,
    pub has_conflict: bool,
    /// Every stored session→pull request link, strongest confidence first. A
    /// link records a reference; it never says the pull request merged.
    pub pr_links: Vec<SessionPrLink>,
    /// The verified parent session, when it resolves to exactly one indexed
    /// user session. Display only: it never changes this
    /// row's membership, order or measurements.
    pub parent: Option<SessionParent>,
    /// Verified opening metadata identifies an automated reviewer. Display only.
    pub automated_review: bool,
    pub cursor: SessionCursor,
}

/// The indexed user session named as a listed session's parent. Present only
/// for accepted structural evidence whose parent host and native
/// identity resolve to exactly one indexed user session other than the child.
/// A parent that is not indexed, or not uniquely, is absent rather than
/// guessed, so no row ever points at the wrong session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionParent {
    /// The parent's canonical identity, the one its own row and detail use.
    pub session_id: String,
    pub host: String,
    /// The parent's saved title, as its own row reads it: blank is absent,
    /// and nothing here derives one from content.
    pub title: Option<String>,
    pub evidence: ParentEvidence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParentEvidence {
    NativeSpawn,
    AgentLaunch,
    NativeReviewer,
}

impl From<crate::creation::CreationEvidence> for ParentEvidence {
    fn from(value: crate::creation::CreationEvidence) -> Self {
        match value {
            crate::creation::CreationEvidence::CodexThreadSpawn => Self::NativeSpawn,
            crate::creation::CreationEvidence::CliArtifactCreate
            | crate::creation::CreationEvidence::CodexClaudeLaunch
            | crate::creation::CreationEvidence::CodexCliLaunch => Self::AgentLaunch,
        }
    }
}

/// One stored link from a listed session to a canonical pull request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionPrLink {
    pub pull_request: crate::pr_link::PrIdentity,
    pub confidence: crate::pr_link::PrConfidence,
    /// The refresh-owned title, when a refresh stored one.
    pub title: Option<String>,
}

/// The hosts a list can be narrowed to. `other` stays listable without a filter.
pub const HOSTS: [&str; 4] = ["claude", "codex", "cursor", "other"];

/// Which indexed user sessions a page lists. Every part is applied in SQL
/// before the cursor and the page bound, so a filter never thins a page that
/// was already cut.
#[derive(Clone, Copy, Debug, Default)]
pub struct SessionFilter<'a> {
    /// Substring of the identity, repository/working directory, branch or
    /// saved title. At most 256 characters.
    pub search: &'a str,
    pub sort: SessionSort,
    /// `None` lists every host. Otherwise a non-empty set of [`HOSTS`];
    /// duplicates collapse.
    pub hosts: Option<&'a [&'a str]>,
    /// Only sessions with at least one stored pull-request link, of any
    /// confidence, independent of merge state or the selected window. A link
    /// to a number GitHub said is not a pull request does not count.
    pub with_prs: bool,
    /// Only the sessions linked to exactly this canonical pull request.
    pub pull_request: Option<PrMembership<'a>>,
}

/// The linked sessions of one canonical pull request: every indexed user
/// session with a stored link to exactly this repository and number, whatever
/// the pull request's cached state and whether or not the session did anything
/// in a selected window. With `confirmed_only`, a session whose link is only
/// `inferred` is not a member. The identity is compared exactly, never by a
/// substring of a title, repository or URL, so the same number in another
/// repository is another pull request. A number GitHub said is not a pull
/// request has no members.
#[derive(Clone, Copy, Debug)]
pub struct PrMembership<'a> {
    pub identity: &'a crate::pr_link::PrIdentity,
    pub confirmed_only: bool,
}

impl SessionFilter<'_> {
    /// The validated host set as one bound value (`|claude|codex|`), or
    /// `None` for every host.
    fn host_set(&self) -> Result<Option<String>> {
        let Some(hosts) = self.hosts else {
            return Ok(None);
        };
        if hosts.is_empty() || hosts.iter().any(|h| !HOSTS.contains(h)) {
            return Err(Error::InvalidInput("invalid session filter"));
        }
        let mut bound = String::from("|");
        for host in HOSTS.iter().filter(|host| hosts.contains(host)) {
            bound.push_str(host);
            bound.push('|');
        }
        Ok(Some(bound))
    }
}
/// The session's own first event (ms) for the session aliased `s`: the
/// earliest of its work events with a stored time, the same rows
/// `v_session_events` holds and M-16's sessions per day counts it on (the
/// shared [`work_record_sql`] predicate). Records a fork shares with the
/// session it was forked from belong to that session and are not counted. The
/// Sessions page orders by this when no start is stored, and both the page and
/// the Dashboard show it as a Claude session's start.
macro_rules! own_first_ms_sql {
    () => {
        concat!(
            "(SELECT min(r.ts_ms) FROM records r WHERE r.session_id=s.session_id AND ",
            work_record_sql!(),
            ")"
        )
    };
}

/// The earliest non-meta time (ms) among the records the session aliased `s`
/// shares with a session it was forked from. It never sets a start: `first_ts`,
/// the earliest visible work, reads it, and a fork with no work of its own yet
/// (no shown start) is ordered by it, beside the history it was forked from,
/// rather than below every dated session.
macro_rules! copied_first_ms_sql {
    () => {
        "(SELECT min(r.ts_ms) FROM native_record_copies m JOIN records r ON r.uuid=m.record_uuid
                    WHERE m.session_id=s.session_id AND r.is_meta=0)"
    };
}

/// The start a session row shows. Claude Code transcripts record no session
/// start, so a Claude session with none stored starts at its own first event;
/// every other host shows only the start it recorded. Nothing is written back.
/// Takes the stored start, host and first-message expressions, so the page can
/// pass the `first_ms` it already computed instead of running the subquery twice.
macro_rules! shown_start_sql {
    ($stored:expr, $host:expr, $first:expr) => {
        concat!(
            "CASE WHEN ",
            $stored,
            " IS NULL AND ",
            $host,
            "='claude' THEN ",
            $first,
            " ELSE ",
            $stored,
            " END"
        )
    };
}

const PAGE_SQL: &str = concat!(
    "WITH candidates AS MATERIALIZED (
              SELECT s.session_id,s.host,coalesce(s.repo,s.cwd) repo,s.git_branch,s.has_conflict,s.started_at_ms native_start,
                nullif(trim(s.title),'') title,
                ",
    own_first_ms_sql!(),
    " own_first_ms,
                ",
    copied_first_ms_sql!(),
    " copied_first_ms,
                (SELECT max(r.ts_ms) FROM records r WHERE r.session_id=s.session_id AND ",
    work_record_sql!(),
    ") last_activity_ms
              FROM sessions s WHERE s.kind='user' AND (?2 IS NULL OR instr(?2,'|'||s.host||'|')>0)
                -- A recorded session→PR link of any confidence, before paging.
                -- A number GitHub said is not a pull request is no link.
                AND (?6=0 OR EXISTS (SELECT 1 FROM pr_links l JOIN pull_requests p ON p.id=l.pr_id
                     WHERE l.session_id=s.session_id AND NOT ",
    crate::not_found_on_github_sql!(),
    "))
                -- A link to exactly one canonical pull request, after the
                -- confidence filter, also before paging.
                AND (?7 IS NULL OR EXISTS (SELECT 1 FROM pr_links l JOIN pull_requests p ON p.id=l.pr_id
                     WHERE l.session_id=s.session_id AND p.repo=?7 AND p.number=?8
                       AND NOT ",
    crate::not_found_on_github_sql!(),
    "
                       AND (?9=0 OR l.confidence<>'inferred')))
                -- One exact identity, or the whole filtered list. A substring
                -- search can match any number of sessions, so a caller that
                -- means one session says which one here rather than searching
                -- for it and hoping it is on the first page.
                AND (?5 IS NULL OR s.session_id=?5)
                AND (?1='' OR instr(lower(coalesce(s.repo,s.cwd,'')),lower(?1))>0
                     OR instr(lower(coalesce(s.git_branch,'')),lower(?1))>0
                     OR instr(lower(s.session_id),lower(?1))>0
                     OR instr(lower(coalesce(s.title,'')),lower(?1))>0)
            ), ranked AS (
              SELECT *,",
    shown_start_sql!("native_start", "host", "own_first_ms"),
    " started_at_ms,
                coalesce(min(own_first_ms,copied_first_ms),own_first_ms,copied_first_ms) first_ms,
                CASE WHEN ?10='recently_active' THEN coalesce(last_activity_ms,-9223372036854775808) ELSE coalesce(native_start,own_first_ms,copied_first_ms,-9223372036854775808) END sort_time FROM candidates
            ), page AS MATERIALIZED (
              SELECT * FROM ranked WHERE ?3 IS NULL OR sort_time < ?3 OR (sort_time=?3 AND session_id < ?4)
              ORDER BY sort_time DESC,session_id DESC LIMIT 51
            ), work AS (
              SELECT session_id,count(*) record_count
              FROM (
                SELECT session_id FROM records
                  WHERE session_id IN (SELECT session_id FROM page) AND is_meta=0
                UNION ALL
                SELECT m.session_id FROM native_record_copies m JOIN records r ON r.uuid=m.record_uuid
                  WHERE m.session_id IN (SELECT session_id FROM page) AND r.is_meta=0
              ) GROUP BY session_id
            )
            SELECT p.session_id,p.host,p.repo,p.git_branch,coalesce(w.record_count,0),p.has_conflict,
              p.sort_time,p.first_ms,p.title,p.started_at_ms,p.last_activity_ms
            FROM page p LEFT JOIN work w ON w.session_id=p.session_id
            ORDER BY p.sort_time DESC,p.session_id DESC LIMIT 51"
);

impl Store {
    pub fn sessions_page(
        &self,
        search: &str,
        host: Option<&str>,
        after: Option<&SessionCursor>,
    ) -> Result<Vec<SessionSummary>> {
        let snapshot = self.connection.unchecked_transaction()?;
        let summaries = page(&snapshot, search, host, after)?;
        snapshot.commit()?;
        Ok(summaries)
    }

    /// [`Store::sessions_page`] with the full filter: a host set and the
    /// recorded-pull-request condition, both applied before paging.
    pub fn sessions_page_filtered(
        &self,
        filter: &SessionFilter<'_>,
        after: Option<&SessionCursor>,
    ) -> Result<Vec<SessionSummary>> {
        let snapshot = self.connection.unchecked_transaction()?;
        let summaries = filtered(&snapshot, filter, after)?;
        snapshot.commit()?;
        Ok(summaries)
    }
}

/// The same bounded metadata page against a caller-owned read snapshot, so a
/// reader can measure exactly these sessions without a second connection or a
/// second snapshot. The caller owns the transaction; this never commits one.
pub fn page(
    connection: &rusqlite::Connection,
    search: &str,
    host: Option<&str>,
    after: Option<&SessionCursor>,
) -> Result<Vec<SessionSummary>> {
    let hosts = host.as_ref().map(std::slice::from_ref);
    listed(
        connection,
        &SessionFilter {
            search,
            sort: SessionSort::Started,
            hosts,
            with_prs: false,
            pull_request: None,
        },
        after,
        None,
    )
}

/// The bounded page under the full filter, against a caller-owned snapshot.
pub fn filtered(
    connection: &rusqlite::Connection,
    filter: &SessionFilter<'_>,
    after: Option<&SessionCursor>,
) -> Result<Vec<SessionSummary>> {
    listed(connection, filter, after, None)
}

/// Exactly the session with this identity, or nothing.
///
/// The same query, the same snapshot and the same metadata as a listed row —
/// one identity instead of a filter. A caller that wants one session must not
/// reach it by searching: the search matches a substring of the identity, so
/// it can return any number of rows, and the one that was asked for need not
/// be among the first page of them.
///
/// An identity is compared exactly. A prefix of another session's identity is
/// a different session and is not found here.
pub fn exact(connection: &rusqlite::Connection, id: &str) -> Result<Option<SessionSummary>> {
    Ok(
        listed(connection, &SessionFilter::default(), None, Some(id))?
            .into_iter()
            .next(),
    )
}

fn listed(
    connection: &rusqlite::Connection,
    filter: &SessionFilter<'_>,
    after: Option<&SessionCursor>,
    id: Option<&str>,
) -> Result<Vec<SessionSummary>> {
    if filter.search.chars().count() > 256 {
        return Err(Error::InvalidInput("invalid session filter"));
    }
    if after.is_some_and(|cursor| cursor.sort != filter.sort) {
        return Err(Error::InvalidInput("invalid session cursor"));
    }
    let hosts = filter.host_set()?;
    let pull_request = filter
        .pull_request
        .map(|member| {
            i64::try_from(member.identity.number())
                .map(|number| (member.identity.repository(), number, member.confirmed_only))
                .map_err(|_| Error::InvalidInput("invalid session filter"))
        })
        .transpose()?;
    let mut statement = connection.prepare(PAGE_SQL)?;
    let mut summaries = statement
        .query_map(
            rusqlite::params![
                filter.search,
                hosts,
                after.map(|a| a.time),
                after.map(|a| a.id.as_str()),
                id,
                filter.with_prs,
                pull_request.map(|(repository, _, _)| repository),
                pull_request.map(|(_, number, _)| number),
                pull_request.is_some_and(|(_, _, confirmed_only)| confirmed_only),
                match filter.sort {
                    SessionSort::Started => "started",
                    SessionSort::RecentlyActive => "recently_active",
                },
            ],
            |row| {
                let id: String = row.get(0)?;
                Ok((
                    SessionSummary {
                        cursor: SessionCursor {
                            time: row.get(6)?,
                            sort: filter.sort,
                            id: id.clone(),
                        },
                        id,
                        host: row.get(1)?,
                        title: row.get(8)?,
                        started_at_ms: row.get(9)?,
                        pr_links: Vec::new(),
                        parent: None,
                        automated_review: false,
                        repo: row.get(2)?,
                        branch: row.get(3)?,
                        first_ts: None,
                        last_activity_at_ms: row.get(10)?,
                        record_count: u64::try_from(row.get::<_, i64>(4)?)
                            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, -1))?,
                        has_conflict: row.get(5)?,
                        model: None,
                        other_models: 0,
                    },
                    row.get::<_, Option<i64>>(7)?,
                ))
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    // Derive visible ranges even for sessions not reimported since an upgrade.
    // At most 51 summaries share one read snapshot; preserve sub-ms ordering
    // through the same timestamp helper used by the writer.
    for (summary, first_ms) in &mut summaries {
        summary.first_ts = crate::read::first_work_timestamp(connection, &summary.id, *first_ms)?;
    }
    let mut summaries: Vec<SessionSummary> =
        summaries.into_iter().map(|(summary, _)| summary).collect();
    let ids: Vec<&str> = summaries.iter().map(|s| s.id.as_str()).collect();
    let mut links = page_links(connection, &ids)?;
    let (mut parents, reviewers) = parents_and_reviewers(connection, &ids)?;
    let models = crate::session_model::whole_sessions(connection, &ids)?;
    for summary in &mut summaries {
        if let Some(most) = models.get(&summary.id).and_then(|uses| uses.most_used()) {
            summary.model = Some(most.model);
            summary.other_models = most.other_models;
        }
        summary.pr_links = links.remove(&summary.id).unwrap_or_default();
        summary.parent = parents.remove(&summary.id);
        summary.automated_review = reviewers.contains(&summary.id);
    }
    Ok(summaries)
}

/// Accepted creation relations of the named children whose parent resolves by exact
/// host and native identity to an indexed user session. The count beside each
/// row says how many user sessions hold that identity; only exactly one is a
/// parent. A relation resting on a Codex agent's CLI launch, of any kind,
/// named its exact canonical parent as well, and resolves only to that
/// session. Keyed lookups on the relation's
/// primary key and the sessions' host/native index. The `IN` list is appended
/// by [`parents`].
pub const PARENT_SQL: &str = "SELECT r.child_session_id,r.evidence_kind,p.session_id,p.host,
       nullif(trim(p.title),''),
       (SELECT count(*) FROM sessions q WHERE q.host=r.parent_host
          AND q.native_session_id=r.parent_native_session_id AND q.kind='user')
     FROM session_creation_relations r
     JOIN sessions c ON c.session_id=r.child_session_id
     JOIN sessions p ON p.host=r.parent_host AND p.native_session_id=r.parent_native_session_id
     WHERE r.state='accepted' AND c.kind='user' AND c.host=r.child_host
       AND c.native_session_id=r.child_native_session_id
       AND p.kind='user' AND p.session_id<>r.child_session_id
       AND (r.evidence_kind NOT IN ('cli_artifact_create','codex_claude_launch','codex_cli_launch')
            OR p.session_id=r.parent_session_id)";

/// The exactly resolved parent of each named session that has one, against a
/// caller-owned read snapshot. At most [`MAX_CONTEXT`] identifiers, which
/// covers a page and a Dashboard lane set. Display only.
pub fn parents(
    connection: &rusqlite::Connection,
    ids: &[&str],
) -> Result<std::collections::HashMap<String, SessionParent>> {
    Ok(parents_and_reviewers(connection, ids)?.0)
}

fn parents_and_reviewers(
    connection: &rusqlite::Connection,
    ids: &[&str],
) -> Result<(
    std::collections::HashMap<String, SessionParent>,
    std::collections::HashSet<String>,
)> {
    let mut unique: Vec<&str> = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() > MAX_CONTEXT {
        return Err(Error::InvalidInput(
            "too many sessions requested for one parent read",
        ));
    }
    let mut parents = std::collections::HashMap::new();
    let mut reviewers = std::collections::HashSet::new();
    if unique.is_empty() {
        return Ok((parents, reviewers));
    }
    let mut sql = String::with_capacity(PARENT_SQL.len() + unique.len() * 5 + 32);
    sql.push_str(PARENT_SQL);
    sql.push_str(" AND r.child_session_id IN (");
    for index in 0..unique.len() {
        if index > 0 {
            sql.push(',');
        }
        sql.push('?');
        sql.push_str(&(index + 1).to_string());
    }
    sql.push(')');
    let rows = connection
        .prepare(&sql)?
        .query_map(rusqlite::params_from_iter(unique.iter().copied()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                SessionParent {
                    evidence: row.get::<_, crate::creation::CreationEvidence>(1)?.into(),
                    session_id: row.get(2)?,
                    host: row.get(3)?,
                    title: row.get(4)?,
                },
                row.get::<_, i64>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (child, parent, holders) in rows {
        if holders == 1 {
            parents.insert(child, parent);
        }
    }
    // These exact tokens are written by the native Guardian header reader.
    // A conflicted origin or creation claim cannot supply a reviewer label.
    let mut reviewer_sql = String::from(
        "SELECT o.session_id,p.session_id,p.host,nullif(trim(p.title),''),
       (SELECT count(*) FROM sessions q WHERE q.host=o.parent_host
          AND q.native_session_id=o.parent_native_session_id AND q.kind='user'),
       EXISTS(SELECT 1 FROM session_creation_relations r WHERE r.child_session_id=o.session_id)
     FROM human_session_origins o
     JOIN sessions c ON c.session_id=o.session_id
     JOIN sessions p ON p.host=o.parent_host AND p.native_session_id=o.parent_native_session_id
     WHERE o.method='native_reviewer_header' AND o.evidence_id='rollout_opening_session_meta'
       AND o.launch_id='native_reviewer_header' AND o.conflicted=0
       AND c.kind='user' AND c.host=o.host AND c.native_session_id=o.native_session_id
       AND p.kind='user' AND p.session_id<>o.session_id
       AND NOT EXISTS(SELECT 1 FROM session_creation_relations r
          WHERE r.child_session_id=o.session_id AND r.state='conflicted')
       AND o.session_id IN (",
    );
    for index in 0..unique.len() {
        if index > 0 {
            reviewer_sql.push(',');
        }
        reviewer_sql.push('?');
        reviewer_sql.push_str(&(index + 1).to_string());
    }
    reviewer_sql.push(')');
    let rows = connection
        .prepare(&reviewer_sql)?
        .query_map(rusqlite::params_from_iter(unique.iter().copied()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                SessionParent {
                    session_id: row.get(1)?,
                    host: row.get(2)?,
                    title: row.get(3)?,
                    evidence: ParentEvidence::NativeReviewer,
                },
                row.get::<_, i64>(4)?,
                row.get::<_, bool>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (child, parent, holders, creation_claim) in rows {
        if holders != 1 {
            continue;
        }
        reviewers.insert(child.clone());
        if !creation_claim {
            parents.entry(child).or_insert(parent);
        }
    }
    Ok((parents, reviewers))
}

/// One statement for every link of a page's sessions, on the same snapshot.
/// Canonical identity is re-validated exactly as every other link read does;
/// a stored row that is not canonical fails rather than being re-spelled.
/// A number GitHub said is not a pull request is not shown as one.
const LINKS_SQL: &str = concat!(
    "SELECT l.session_id,p.repo,p.number,p.url,l.confidence,p.title
     FROM pr_links l JOIN pull_requests p ON p.id=l.pr_id WHERE NOT ",
    crate::not_found_on_github_sql!(),
    " AND l.session_id IN "
);

fn page_links(
    connection: &rusqlite::Connection,
    ids: &[&str],
) -> Result<std::collections::HashMap<String, Vec<SessionPrLink>>> {
    let mut links = std::collections::HashMap::<String, Vec<SessionPrLink>>::new();
    if ids.is_empty() {
        return Ok(links);
    }
    let mut sql = String::from(LINKS_SQL);
    sql.push('(');
    for index in 0..ids.len() {
        if index > 0 {
            sql.push(',');
        }
        sql.push('?');
        sql.push_str(&(index + 1).to_string());
    }
    sql.push(')');
    let rows = connection
        .prepare(&sql)?
        .query_map(rusqlite::params_from_iter(ids), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, crate::pr_link::PrConfidence>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (session, repo, number, url, confidence, title) in rows {
        links.entry(session).or_default().push(SessionPrLink {
            pull_request: crate::pr_link::PrIdentity::from_stored(&repo, number, &url)?,
            confidence,
            title,
        });
    }
    for list in links.values_mut() {
        list.sort_by(|a, b| {
            b.confidence
                .rank()
                .cmp(&a.confidence.rank())
                .then_with(|| a.pull_request.cmp(&b.pull_request))
        });
    }
    Ok(links)
}

/// What one indexed session ran against: the host that recorded it, the
/// repository and branch it was working in, the title a source saved, its
/// start (see [`SessionContext::started_at_ms`]) and how many pull requests it has recorded links to.
/// Every descriptive field is nullable, because a history can simply not
/// carry it; the link counts are measured over the stored index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionContext {
    pub id: String,
    pub host: String,
    pub repo: Option<String>,
    pub branch: Option<String>,
    /// The same saved title the Sessions page shows: blank reads as absent,
    /// and nothing here derives one from content.
    pub title: Option<String>,
    pub automated_review: bool,
    /// The same start the Sessions page shows: the start the host recorded,
    /// or, for a Claude session with none stored, its own first event (never a
    /// record copied from a session it was forked from).
    pub started_at_ms: Option<i64>,
    /// Distinct canonical pull requests this session has a stored link to, at
    /// every evidence level, whatever their merge state, refresh state or the
    /// time the link was observed, except numbers GitHub said are not pull
    /// requests. Zero means no link is recorded, not that no pull request
    /// exists.
    pub pr_links: u64,
    /// The part of `pr_links` whose evidence is only `inferred`.
    pub inferred_pr_links: u64,
    /// The exactly resolved session that created this one; see
    /// [`SessionParent`].
    pub parent: Option<SessionParent>,
    /// Stored positive evidence says another session created this one: an
    /// accepted creation relation, an unconflicted saved Human session origin
    /// or an accepted child fact ([`crate::child_fact`]), bound to exactly
    /// this session's canonical ID, host and native ID. The first two are the
    /// facts the Human input view uses to treat the session's user messages
    /// as an agent's; a child fact alone does not change that view, so a
    /// known child is not always one the Human view excludes. Independent of
    /// `parent`: a known child's parent can still be unindexed, ambiguous or
    /// unknown. A conflicted, withheld or mismatched fact alone never sets
    /// it. Display only.
    pub known_child: bool,
    /// What a list may show of the session: [`ChildCheck::Child`] exactly
    /// when `known_child`, else whether its supported checks have finished
    /// for what the index holds of it now ([`crate::child_check`]). Display
    /// only; a finished check never says a person created the session.
    pub check: ChildCheck,
}

pub use crate::child_check::ChildCheck;

/// Identifiers one context read accepts. A caller names sessions it already
/// holds, so this bound is a guard on the generated `IN` list, not a page size.
pub const MAX_CONTEXT: usize = 200;

/// The session domain the shared projections use, and the same repository
/// and saved-title expressions the page projects: a recorded repository, else
/// the working directory it ran in; a nonblank saved title; the start the page
/// shows, including a Claude session's own first event when it has no stored
/// start. Link counts are
/// keyed lookups on `pr_links`' own (session_id, pr_id) primary key, so each
/// canonical pull request counts once per session and the read is bounded by
/// the named sessions, not by the number of stored links. The join is the one
/// the page's link read uses. The known-child bit repeats the creation and
/// origin join conditions of `v_human_inputs` exactly, as keyed lookups on
/// each table's own session key, and also reads accepted child facts, which
/// that view does not. The finished-check bit is one keyed lookup of the
/// session's own check row. The `IN` list is appended by [`context`].
pub const CONTEXT_SQL: &str = concat!(
    "SELECT s.session_id,s.host,coalesce(s.repo,s.cwd),s.git_branch,
       nullif(trim(s.title),''),",
    shown_start_sql!("s.started_at_ms", "s.host", own_first_ms_sql!()),
    ",
       (SELECT count(*) FROM pr_links l JOIN pull_requests p ON p.id=l.pr_id
          WHERE l.session_id=s.session_id AND NOT ",
    crate::not_found_on_github_sql!(),
    "),
       (SELECT count(*) FROM pr_links l JOIN pull_requests p ON p.id=l.pr_id
          WHERE l.session_id=s.session_id AND l.confidence='inferred' AND NOT ",
    crate::not_found_on_github_sql!(),
    "),
       ",
    crate::child_check::known_child_sql!(),
    ",
       ",
    crate::child_check::checked_sql!(),
    "
     FROM sessions s WHERE s.kind='user'"
);

/// Content-free context for exactly the named sessions, against a caller-owned
/// read snapshot, so a reader can measure the same sessions in one snapshot.
///
/// Rows come back in identifier order. Duplicate identifiers collapse, and an
/// identifier no indexed user session owns is simply absent from the result
/// rather than reported as an empty context.
pub fn context(connection: &rusqlite::Connection, ids: &[&str]) -> Result<Vec<SessionContext>> {
    let ids: Vec<&str> = {
        let mut unique: Vec<&str> = ids.to_vec();
        unique.sort_unstable();
        unique.dedup();
        unique
    };
    if ids.len() > MAX_CONTEXT {
        return Err(Error::InvalidInput(
            "too many sessions requested for one context read",
        ));
    }
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut sql = String::with_capacity(CONTEXT_SQL.len() + ids.len() * 5 + 24);
    sql.push_str(CONTEXT_SQL);
    sql.push_str(" AND s.session_id IN (");
    for index in 0..ids.len() {
        if index > 0 {
            sql.push(',');
        }
        sql.push('?');
        sql.push_str(&(index + 1).to_string());
    }
    sql.push_str(") ORDER BY s.session_id");
    let mut statement = connection.prepare(&sql)?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(ids), |row| {
            let count = |index: usize| {
                let value: i64 = row.get(index)?;
                u64::try_from(value)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
            };
            let known_child: bool = row.get(8)?;
            Ok(SessionContext {
                id: row.get(0)?,
                host: row.get(1)?,
                repo: row.get(2)?,
                branch: row.get(3)?,
                title: row.get(4)?,
                automated_review: false,
                started_at_ms: row.get(5)?,
                pr_links: count(6)?,
                inferred_pr_links: count(7)?,
                parent: None,
                known_child,
                check: if known_child {
                    ChildCheck::Child
                } else if row.get(9)? {
                    ChildCheck::Checked
                } else {
                    ChildCheck::Checking
                },
            })
        })?
        .collect::<std::result::Result<Vec<SessionContext>, _>>()?;
    drop(statement);
    let named: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    let (mut parents, reviewers) = parents_and_reviewers(connection, &named)?;
    let mut rows = rows;
    for row in &mut rows {
        row.parent = parents.remove(&row.id);
        row.automated_review = reviewers.contains(&row.id);
    }
    Ok(rows)
}

// Keep in step with PARENT_SQL and the reviewer read in parents_and_reviewers:
// a new parent source added there must be added here, or its children are
// never found under a listed parent.
/// Sessions that may have been created by one of the named sessions, with
/// their last recorded time: every user session an accepted creation relation
/// or an unconflicted native reviewer origin names as created by the named
/// session's host and native identity. These are candidates only: a caller
/// keeps a candidate only when [`parents`] resolves its parent to one of the
/// named sessions, the same verified link every list groups by. The relation
/// side is a keyed lookup on its parent index; the origin side reads the
/// reviewer origins, a table of at most one row per session. At most
/// [`MAX_CONTEXT`] identifiers.
pub fn child_candidates(
    connection: &rusqlite::Connection,
    ids: &[&str],
) -> Result<Vec<(String, Option<String>)>> {
    let mut unique: Vec<&str> = ids.to_vec();
    unique.sort_unstable();
    unique.dedup();
    if unique.len() > MAX_CONTEXT {
        return Err(Error::InvalidInput(
            "too many sessions requested for one child read",
        ));
    }
    if unique.is_empty() {
        return Ok(Vec::new());
    }
    let mut list = String::with_capacity(unique.len() * 5);
    for index in 0..unique.len() {
        if index > 0 {
            list.push(',');
        }
        list.push('?');
        list.push_str(&(index + 1).to_string());
    }
    let sql = format!(
        "SELECT c.session_id,c.last_ts FROM sessions p
           JOIN session_creation_relations r ON r.parent_host=p.host
             AND r.parent_native_session_id=p.native_session_id AND r.state='accepted'
           JOIN sessions c ON c.session_id=r.child_session_id AND c.kind='user'
           WHERE p.kind='user' AND p.session_id IN ({list})
         UNION
         SELECT c.session_id,c.last_ts FROM sessions p
           JOIN human_session_origins o ON o.parent_host=p.host
             AND o.parent_native_session_id=p.native_session_id
             AND o.method='native_reviewer_header' AND o.conflicted=0
           JOIN sessions c ON c.session_id=o.session_id AND c.kind='user'
           WHERE p.kind='user' AND p.session_id IN ({list})"
    );
    let rows = connection
        .prepare(&sql)?
        .query_map(rusqlite::params_from_iter(unique.iter().copied()), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod query_plan_tests {
    use super::*;

    /// The planned steps for one set of bindings, whatever they select.
    fn plan(store: &Store, id: Option<&str>) -> Vec<String> {
        plan_with(store, id, false)
    }

    fn plan_with(store: &Store, id: Option<&str>, with_prs: bool) -> Vec<String> {
        plan_bound(store, id, with_prs, None, SessionSort::Started)
    }

    fn plan_bound(
        store: &Store,
        id: Option<&str>,
        with_prs: bool,
        pull_request: Option<(&str, i64, bool)>,
        sort: SessionSort,
    ) -> Vec<String> {
        let mut statement = store
            .connection
            .prepare(&format!("EXPLAIN QUERY PLAN {PAGE_SQL}"))
            .unwrap();
        statement
            .query_map(
                rusqlite::params![
                    "no-match",
                    None::<String>,
                    None::<i64>,
                    None::<String>,
                    id,
                    with_prs,
                    pull_request.map(|(repository, _, _)| repository),
                    pull_request.map(|(_, number, _)| number),
                    pull_request.is_some_and(|(_, _, confirmed)| confirmed),
                    match sort {
                        SessionSort::Started => "started",
                        SessionSort::RecentlyActive => "recently_active",
                    },
                ],
                |row| row.get::<_, String>(3),
            )
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap()
    }

    fn scans_every_record(plan: &[String]) -> bool {
        plan.iter().any(|step| {
            step == "SCAN records"
                || step.starts_with("SCAN records ")
                || step == "SCAN r"
                || step.starts_with("SCAN r ")
        })
    }

    #[test]
    fn filtered_pages_do_not_plan_full_record_scans() {
        let store = Store::open_in_memory().unwrap();
        let plan = plan(&store, None);
        assert!(
            plan.iter().any(|step| step.starts_with("SEARCH records ")),
            "{plan:?}"
        );
        assert!(!scans_every_record(&plan), "{plan:?}");
    }

    #[test]
    fn recent_pages_do_not_plan_full_record_scans() {
        let store = Store::open_in_memory().unwrap();
        let plan = plan_bound(&store, None, false, None, SessionSort::RecentlyActive);
        assert!(!scans_every_record(&plan), "{plan:?}");
    }

    #[test]
    fn an_exact_identity_does_not_plan_full_record_scans_either() {
        // The exact lookup shares this query, so it shares this guarantee:
        // naming one session must not cost a scan of every record.
        let store = Store::open_in_memory().unwrap();
        let plan = plan(&store, Some("session-one"));
        assert!(
            plan.iter().any(|step| step.starts_with("SEARCH records ")),
            "{plan:?}"
        );
        assert!(!scans_every_record(&plan), "{plan:?}");
    }

    #[test]
    fn the_context_link_counts_are_keyed_lookups() {
        // The Dashboard names at most a capped set of sessions; counting their
        // links must go through the link key and the session key, never a scan
        // of every stored link or every session.
        let store = Store::open_in_memory().unwrap();
        let plan: Vec<String> = store
            .connection
            .prepare(&format!(
                "EXPLAIN QUERY PLAN {CONTEXT_SQL} AND s.session_id IN (?1,?2)"
            ))
            .unwrap()
            .query_map(["a", "b"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(
            plan.iter()
                .filter(|step| step.starts_with("SEARCH l "))
                .count()
                >= 2,
            "{plan:?}"
        );
        // The known-child bit is one keyed lookup per named session in each
        // of its tables, and so is the finished-check bit, never a scan.
        assert!(
            ["c", "o", "f", "k"].iter().all(|alias| plan
                .iter()
                .any(|step| step.starts_with(&format!("SEARCH {alias} ")))),
            "{plan:?}"
        );
        assert!(
            !plan.iter().any(|step| ["l", "s", "p", "c", "o", "f", "k"]
                .iter()
                .any(|alias| step == &format!("SCAN {alias}")
                    || step.starts_with(&format!("SCAN {alias} ")))),
            "{plan:?}"
        );
    }

    #[test]
    fn the_pr_condition_and_the_page_links_are_keyed_lookups() {
        // `EXISTS` is decided per candidate session through the link table's
        // own (session_id, pr_id) key, never by scanning every link, and the
        // page's links are read the same way: bounded by the page, not by the
        // number of stored links.
        let store = Store::open_in_memory().unwrap();
        let plan = plan_with(&store, None, true);
        assert!(!scans_every_record(&plan), "{plan:?}");
        assert!(
            !plan
                .iter()
                .any(|step| step == "SCAN l" || step.starts_with("SCAN l ")),
            "{plan:?}"
        );
        let links: Vec<String> = store
            .connection
            .prepare(&format!("EXPLAIN QUERY PLAN {LINKS_SQL}(?1,?2)"))
            .unwrap()
            .query_map(["a", "b"], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(
            links.iter().any(|step| step.starts_with("SEARCH l ")),
            "{links:?}"
        );
    }

    #[test]
    fn the_exact_pull_request_condition_is_keyed_lookups() {
        // One pull request's membership is decided per candidate session
        // through the link key and the pull request's own (repo, number) key:
        // never a scan of every stored link, every pull request or every record.
        let store = Store::open_in_memory().unwrap();
        for confirmed in [false, true] {
            let plan = plan_bound(
                &store,
                None,
                false,
                Some(("xtrace/app", 7, confirmed)),
                SessionSort::Started,
            );
            assert!(!scans_every_record(&plan), "{plan:?}");
            // (`p` is also the outer page's alias, which is scanned by design.)
            assert!(
                plan.iter()
                    .any(|step| step.starts_with("SEARCH p ")
                        && step.ends_with("(repo=? AND number=?)")),
                "{plan:?}"
            );
            assert!(
                plan.iter().any(|step| step.starts_with("SEARCH l ")
                    && step.ends_with("(session_id=? AND pr_id=?)")),
                "{plan:?}"
            );
            assert!(
                !plan
                    .iter()
                    .any(|step| step == "SCAN l" || step.starts_with("SCAN l ")),
                "{plan:?}"
            );
        }
    }
}
