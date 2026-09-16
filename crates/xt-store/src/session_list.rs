//! Content-free, bounded session metadata for the initial Sessions browser.
use crate::{Error, Result, Store};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionCursor {
    pub time: i64,
    pub id: String,
}
#[derive(Clone, Debug)]
pub struct SessionSummary {
    pub id: String,
    pub host: String,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub first_ts: Option<String>,
    pub record_count: u64,
    pub has_conflict: bool,
    pub cursor: SessionCursor,
}
impl Store {
    pub fn sessions_page(
        &self,
        search: &str,
        host: Option<&str>,
        after: Option<&SessionCursor>,
    ) -> Result<Vec<SessionSummary>> {
        if search.chars().count() > 256
            || host.is_some_and(|h| !matches!(h, "claude" | "codex" | "cursor" | "other"))
        {
            return Err(Error::InvalidInput("invalid session filter"));
        }
        let mut statement = self.connection.prepare(
            "WITH selected AS (
              SELECT session_id,host,coalesce(repo,cwd) repo,git_branch,first_ts,(SELECT count(*) FROM session_work_records m JOIN records r ON r.uuid=m.record_uuid WHERE m.session_id=sessions.session_id AND r.is_meta=0) record_count,has_conflict,
                coalesce(started_at_ms,cast((julianday(first_ts)-2440587.5)*86400000 AS INTEGER),-9223372036854775808) sort_time
              FROM sessions WHERE kind='user' AND (?2 IS NULL OR host=?2)
                AND (?1='' OR instr(lower(coalesce(repo,cwd,'')),lower(?1))>0
                     OR instr(lower(coalesce(git_branch,'')),lower(?1))>0
                     OR instr(lower(session_id),lower(?1))>0)
            )
            SELECT session_id,host,repo,git_branch,first_ts,record_count,has_conflict,sort_time,
              (SELECT CASE WHEN count(DISTINCT model)>1 THEN 'Multiple models' ELSE min(model) END
               FROM records JOIN session_work_records m ON m.record_uuid=records.uuid
                WHERE m.session_id=selected.session_id AND records.is_meta=0)
            FROM selected WHERE ?3 IS NULL OR sort_time < ?3 OR (sort_time=?3 AND session_id < ?4)
            ORDER BY sort_time DESC,session_id DESC LIMIT 51")?;
        statement
            .query_map(
                rusqlite::params![
                    search,
                    host,
                    after.map(|a| a.time),
                    after.map(|a| a.id.as_str())
                ],
                |row| {
                    let id: String = row.get(0)?;
                    Ok(SessionSummary {
                        cursor: SessionCursor {
                            time: row.get(7)?,
                            id: id.clone(),
                        },
                        id,
                        host: row.get(1)?,
                        repo: row.get(2)?,
                        branch: row.get(3)?,
                        first_ts: row.get(4)?,
                        record_count: u64::try_from(row.get::<_, i64>(5)?)
                            .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(5, -1))?,
                        has_conflict: row.get(6)?,
                        model: row.get(8)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
}
