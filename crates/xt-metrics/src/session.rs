//! Per-session measurements over one explicit event window.
//!
//! Every number here is produced by the rule the Dashboard already uses: the
//! same `v_response_usage` projection (M-04 global response selection), the
//! same `v_session_events` projection (copied native records are already
//! reduced to one canonical owner), the same M-02 stored classification and the
//! same M-05 gap fold. Selection and deduplication happen inside those shared
//! projections, before this module restricts rows to a session and a window,
//! so a session total is a slice of the global total and never a re-derivation.
use crate::{Error, MetricsDb, Result, TokenSummary, Window, spans, tokens};
use serde::Serialize;
use std::collections::BTreeMap;
use xt_store::timestamp::{self, InstantKey};

/// Requested identifiers per read. The Sessions page asks for at most one page
/// (51) at a time; the bound keeps the generated `IN` list small and explicit.
pub const MAX_SESSIONS: usize = 200;

/// The same session domain the shared projections use: judge sessions are not
/// part of the metrics and are therefore not measurable identifiers here. The
/// raw host/surface travel with it so a per-session reader can ask about a
/// session's surface without restating that domain.
pub(crate) const EXISTS_QUERY: &str =
    "SELECT session_id,host,surface FROM sessions WHERE kind='user'";
pub(crate) const EVENTS_QUERY: &str = "SELECT session_id,ts_ms,ts,uuid,human_is_eligible,tool_use_count FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";

/// Sessions a sub-session walk visits at most, found or counted.
pub const SUB_SESSION_WALK: usize = 20_000;

/// The verified sub-sessions [`MetricsDb::sub_sessions`] found.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SubSessions {
    /// Within the limit, nearer levels first, each most recently active first.
    pub sessions: Vec<String>,
    /// Found beyond the limit, by the named session whose walk found them.
    pub not_shown: BTreeMap<String, u64>,
    /// Named sessions whose walk stopped at [`SUB_SESSION_WALK`], so their
    /// `not_shown` count is only a lower bound and may be missing.
    pub cut_short: std::collections::BTreeSet<String>,
}

/// What one indexed session contributes inside the selected window.
/// `None` is an unmeasured observation; a real zero stays zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionWindow {
    /// No indexed user session owns this identifier, so nothing was measured.
    /// This is distinct from an indexed session with no events in the window.
    Missing,
    /// The session is indexed. Its counters describe the selected window only;
    /// an empty window is an honest zero for events, human messages and agent
    /// time, while token counters follow M-04 and stay unknown without a
    /// selected response.
    Indexed {
        /// M-01 canonical work events inside the window.
        events: u64,
        /// M-02 human messages; unknown when a record's classification is.
        human_messages: Option<u64>,
        /// Tool-use blocks: the same checked nullable sum of the canonical
        /// `tool_use_count` the Dashboard's tool calls add up, restricted to
        /// this session. Unknown when any in-window record's count is.
        tool_calls: Option<u64>,
        /// M-04 over this session's globally selected responses.
        tokens: TokenSummary,
        /// M-05 active-span milliseconds for this session alone.
        agent_ms: u64,
    },
}

struct Session {
    events: u64,
    human_messages: Option<u64>,
    tool_calls: Option<u64>,
    timeline: Vec<(i64, InstantKey, String)>,
    tokens: tokens::Totals,
}
impl Default for Session {
    /// An indexed session starts at a measured zero. Only an unclassified
    /// record turns its human-message count back into an unknown.
    fn default() -> Self {
        Self {
            events: 0,
            human_messages: Some(0),
            tool_calls: Some(0),
            timeline: Vec::new(),
            tokens: tokens::Totals::default(),
        }
    }
}

/// Bind the window, then one placeholder per requested identifier. The shared
/// projection's own selection is unaffected: it is evaluated before this filter.
pub(crate) fn by_session(query: &str, count: usize) -> String {
    by_session_from(query, count, 3)
}

/// [`by_session`] for a statement whose identifiers are bound from `?first`.
pub(crate) fn by_session_from(query: &str, count: usize, first: usize) -> String {
    let mut sql = String::with_capacity(query.len() + count * 5 + 24);
    sql.push_str(query);
    sql.push_str(if query.contains(" WHERE ") {
        " AND session_id IN ("
    } else {
        " WHERE session_id IN ("
    });
    for index in 0..count {
        if index > 0 {
            sql.push(',');
        }
        sql.push('?');
        sql.push_str(&(index + first).to_string());
    }
    sql.push(')');
    sql
}

pub(crate) fn bind(window: Window, ids: &[&str]) -> Result<Vec<rusqlite::types::Value>> {
    let mut parameters = vec![
        rusqlite::types::Value::Integer(window.start_ms()),
        rusqlite::types::Value::Integer(window.candidate_end_ms()?),
    ];
    parameters.extend(
        ids.iter()
            .map(|id| rusqlite::types::Value::Text((*id).to_owned())),
    );
    Ok(parameters)
}

impl MetricsDb {
    /// M-02, M-04 and M-05 for exactly the requested sessions over one explicit
    /// window. Duplicate identifiers collapse; an unknown identifier reports
    /// [`SessionWindow::Missing`] rather than an invented empty measurement.
    ///
    /// Call this inside [`MetricsDb::read_snapshot`] together with the session
    /// metadata read, so both describe the same committed state while the
    /// native writer keeps appending on its own connection.
    pub fn session_windows(
        &self,
        window: Window,
        sessions: &[&str],
    ) -> Result<BTreeMap<String, SessionWindow>> {
        let ids: Vec<&str> = {
            let mut unique: Vec<&str> = sessions.to_vec();
            unique.sort_unstable();
            unique.dedup();
            unique
        };
        if ids.len() > MAX_SESSIONS {
            return Err(Error::TooManySessions);
        }
        let mut selected = BTreeMap::<String, SessionWindow>::new();
        if ids.is_empty() {
            return Ok(selected);
        }
        let parameters = bind(window, &ids)?;
        // Whether a session exists at all is a metadata fact, read before any
        // measurement so an empty window cannot be mistaken for an absent one.
        let mut indexed = BTreeMap::<String, Session>::new();
        // Every statement shares one parameter vector; the existence read simply
        // does not refer to the two window bounds.
        let mut statement = self
            .connection
            .prepare(&by_session(EXISTS_QUERY, ids.len()))?;
        let mut rows = statement.query(rusqlite::params_from_iter(&parameters))?;
        while let Some(row) = rows.next()? {
            indexed.insert(row.get(0)?, Session::default());
        }
        drop(rows);
        drop(statement);
        for id in &ids {
            if !indexed.contains_key(*id) {
                selected.insert((*id).to_owned(), SessionWindow::Missing);
            }
        }
        if indexed.is_empty() {
            return Ok(selected);
        }

        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let mut statement = self
            .connection
            .prepare(&by_session(EVENTS_QUERY, ids.len()))?;
        let mut rows = statement.query(rusqlite::params_from_iter(&parameters))?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(2)?;
            let instant = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if instant < start || instant >= end {
                continue;
            }
            let id: String = row.get(0)?;
            let Some(session) = indexed.get_mut(&id) else {
                continue;
            };
            session.events += 1;
            match row.get::<_, Option<bool>>(4)? {
                Some(true) => {
                    session.human_messages = match session.human_messages {
                        Some(count) => Some(count.checked_add(1).ok_or(Error::CounterOverflow)?),
                        None => None,
                    }
                }
                Some(false) => {}
                None => session.human_messages = None,
            }
            session.tool_calls =
                crate::counts::add(session.tool_calls, crate::counts::counter(row, 5)?)?;
            session.timeline.push((row.get(1)?, instant, row.get(3)?));
        }
        drop(rows);
        drop(statement);

        let mut statement = self
            .connection
            .prepare(&by_session(tokens::QUERY, ids.len()))?;
        let mut rows = statement.query(rusqlite::params_from_iter(&parameters))?;
        while let Some(row) = rows.next()? {
            let raw: String = row.get(4)?;
            let instant = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        4,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if instant < start || instant >= end {
                continue;
            }
            let id: String = row.get(0)?;
            let values = tokens::counters(row)?;
            if let Some(session) = indexed.get_mut(&id) {
                session.tokens.push(&id, values)?;
            }
        }
        drop(rows);
        drop(statement);

        for (id, mut session) in indexed {
            session.timeline.sort_unstable();
            let agent_ms = if session.timeline.is_empty() {
                0
            } else {
                spans::fold(&session.timeline, |_, _| {})?
            };
            selected.insert(
                id,
                SessionWindow::Indexed {
                    events: session.events,
                    human_messages: session.human_messages,
                    tool_calls: session.tool_calls,
                    tokens: session.tokens.finish()?,
                    agent_ms,
                },
            );
        }
        Ok(selected)
    }

    /// The raw `(host, surface)` of exactly the named indexed user sessions,
    /// bounded like [`MetricsDb::session_windows`]. Metadata only.
    pub(crate) fn session_surfaces(
        &self,
        sessions: &[&str],
    ) -> Result<BTreeMap<String, (String, Option<String>)>> {
        if sessions.len() > MAX_SESSIONS {
            return Err(Error::TooManySessions);
        }
        let mut owned = BTreeMap::new();
        if sessions.is_empty() {
            return Ok(owned);
        }
        // The window bounds are bound but unused, as in the existence read.
        let parameters = bind(Window::new(0, 1)?, sessions)?;
        let mut statement = self
            .connection
            .prepare(&by_session(EXISTS_QUERY, sessions.len()))?;
        let mut rows = statement.query(rusqlite::params_from_iter(&parameters))?;
        while let Some(row) = rows.next()? {
            owned.insert(row.get(0)?, (row.get(1)?, row.get(2)?));
        }
        Ok(owned)
    }

    /// The repository/branch context of exactly the named sessions, read on
    /// this read-only connection so a caller can pair it with
    /// [`MetricsDb::session_windows`] inside one snapshot. Content-free
    /// metadata only; nothing here measures anything.
    pub fn session_context(
        &self,
        sessions: &[&str],
    ) -> Result<Vec<xt_store::session_list::SessionContext>> {
        Ok(xt_store::session_list::context(&self.connection, sessions)?)
    }

    /// Every verified sub-session under the named sessions, at any depth and
    /// whenever it ran, that is not itself one of them: walked level by level
    /// through [`xt_store::session_list::child_candidates`], keeping a
    /// candidate only when its resolved parent is a session of the level
    /// above — the same verified link every list groups by. Each level is
    /// ordered most recently active first.
    ///
    /// At most `limit` are returned, nearer levels first. Every one found
    /// beyond that is counted under the named session whose walk found it, so
    /// a cap is never silent. The walk itself stops after [`SUB_SESSION_WALK`]
    /// sessions; a named session whose walk that cut short is in `cut_short`,
    /// and its count is then only a lower bound.
    pub fn sub_sessions(&self, sessions: &[&str], limit: usize) -> Result<SubSessions> {
        self.sub_session_walk(sessions, limit, SUB_SESSION_WALK)
    }

    /// [`MetricsDb::sub_sessions`] with its walk ceiling as a parameter.
    fn sub_session_walk(
        &self,
        sessions: &[&str],
        limit: usize,
        ceiling: usize,
    ) -> Result<SubSessions> {
        let mut seen: std::collections::BTreeSet<String> =
            sessions.iter().map(|id| (*id).to_owned()).collect();
        // Each found session's origin: the named session whose walk found it.
        let mut origin: BTreeMap<String, String> =
            seen.iter().map(|id| (id.clone(), id.clone())).collect();
        let mut found = SubSessions::default();
        let mut walked = 0usize;
        let mut level: Vec<String> = seen.iter().cloned().collect();
        while !level.is_empty() && walked < ceiling {
            let members: std::collections::BTreeSet<&str> =
                level.iter().map(String::as_str).collect();
            let mut candidates = Vec::new();
            for batch in level.chunks(xt_store::session_list::MAX_CONTEXT) {
                let batch: Vec<&str> = batch.iter().map(String::as_str).collect();
                candidates.extend(xt_store::session_list::child_candidates(
                    &self.connection,
                    &batch,
                )?);
            }
            candidates.retain(|(id, _)| !seen.contains(id));
            candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            candidates.dedup_by(|a, b| a.0 == b.0);
            let mut next = Vec::new();
            for batch in candidates.chunks(xt_store::session_list::MAX_CONTEXT) {
                let ids: Vec<&str> = batch.iter().map(|(id, _)| id.as_str()).collect();
                let parents = xt_store::session_list::parents(&self.connection, &ids)?;
                for (id, _) in batch {
                    let Some(parent) = parents.get(id) else {
                        continue;
                    };
                    if !members.contains(parent.session_id.as_str()) {
                        continue;
                    }
                    let root = origin[&parent.session_id].clone();
                    if walked >= ceiling {
                        // Found but never counted: this walk is cut short.
                        found.cut_short.insert(root);
                        continue;
                    }
                    walked += 1;
                    seen.insert(id.clone());
                    if found.sessions.len() < limit {
                        found.sessions.push(id.clone());
                    } else {
                        *found.not_shown.entry(root.clone()).or_default() += 1;
                    }
                    origin.insert(id.clone(), root);
                    next.push(id.clone());
                }
            }
            level = next;
        }
        // Stopped at the ceiling with sessions whose own sub-sessions were
        // never looked for: their walks are cut short too.
        for id in &level {
            found.cut_short.insert(origin[id].clone());
        }
        Ok(found)
    }

    /// Exactly the session with this identity, or nothing, read on this
    /// read-only connection so the caller can measure it in the same snapshot
    /// it was read in. The same metadata a listed row carries; the identity is
    /// compared exactly, never searched for.
    pub fn session_exact(
        &self,
        id: &str,
    ) -> Result<Option<xt_store::session_list::SessionSummary>> {
        Ok(xt_store::session_list::exact(&self.connection, id)?)
    }

    /// The bounded page under the full filter — host set and recorded pull
    /// requests, both applied before paging — on this read-only connection.
    pub fn sessions_page_filtered(
        &self,
        filter: &xt_store::session_list::SessionFilter<'_>,
        after: Option<&xt_store::session_list::SessionCursor>,
    ) -> Result<Vec<xt_store::session_list::SessionSummary>> {
        Ok(xt_store::session_list::filtered(
            &self.connection,
            filter,
            after,
        )?)
    }

    /// The bounded, content-free session metadata page, read on this read-only
    /// connection so the caller can measure the same rows in one snapshot.
    /// Pagination, search and host filtering are the store's, unchanged.
    pub fn sessions_page(
        &self,
        search: &str,
        host: Option<&str>,
        after: Option<&xt_store::session_list::SessionCursor>,
    ) -> Result<Vec<xt_store::session_list::SessionSummary>> {
        Ok(xt_store::session_list::page(
            &self.connection,
            search,
            host,
            after,
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_walk_cut_short_at_its_ceiling_says_so_for_that_session_only() {
        use xt_store::{
            Host, SessionMeta, SessionSource,
            creation::{
                CODEX_THREAD_SPAWN_VERSION, CreationEvidence, CreationWitness, SessionCreationProof,
            },
        };
        let spawn = |child: &str, parent: &str| SessionCreationProof {
            child_session_id: format!("codex-{child}"),
            child_host: Host::Codex,
            child_native_session_id: child.into(),
            parent_host: Host::Codex,
            parent_native_session_id: parent.into(),
            evidence_kind: CreationEvidence::CodexThreadSpawn,
            evidence_version: CODEX_THREAD_SPAWN_VERSION,
            witness: CreationWitness::RolloutOpeningSessionMeta,
        };
        let mut db = xt_fixtures::TempDb::empty().unwrap();
        for native in ["big", "a", "b", "c", "small"] {
            let mut session =
                SessionMeta::new(format!("codex-{native}"), "codex", SessionSource::Fixture);
            session.native_session_id = Some(native.into());
            db.store_mut().upsert_session(&session, false).unwrap();
        }
        db.store_mut()
            .record_session_creations(&[spawn("a", "big"), spawn("b", "big"), spawn("c", "a")], 1)
            .unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let ids = ["codex-big", "codex-small"];
        let whole = metrics.sub_session_walk(&ids, 10, 10).unwrap();
        assert_eq!(whole.sessions.len(), 3);
        assert!(whole.cut_short.is_empty());
        // Two visits allowed: the third session is never reached, and only
        // the walk it belongs to is marked.
        let cut = metrics.sub_session_walk(&ids, 10, 2).unwrap();
        assert_eq!(cut.sessions.len(), 2);
        assert_eq!(
            cut.cut_short.into_iter().collect::<Vec<_>>(),
            ["codex-big".to_owned()]
        );
    }
    #[test]
    fn session_filter_binds_after_the_window_parameters() {
        assert_eq!(
            by_session("SELECT x FROM t WHERE ts_ms>=?1 AND ts_ms<?2", 2),
            "SELECT x FROM t WHERE ts_ms>=?1 AND ts_ms<?2 AND session_id IN (?3,?4)"
        );
        assert_eq!(
            by_session("SELECT session_id FROM sessions WHERE kind='user'", 1),
            "SELECT session_id FROM sessions WHERE kind='user' AND session_id IN (?3)"
        );
    }
    #[test]
    fn requested_sessions_are_bounded() {
        let db = xt_fixtures::TempDb::empty().unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let window = Window::new(0, 1000).unwrap();
        let many: Vec<String> = (0..=MAX_SESSIONS).map(|i| format!("s{i}")).collect();
        let ids: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(metrics.session_windows(window, &ids).is_err());
        assert!(metrics.session_windows(window, &ids[1..]).is_ok());
        assert!(metrics.session_windows(window, &[]).unwrap().is_empty());
    }
}
