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
        let snapshot = self.connection.unchecked_transaction()?;
        let mut statement = snapshot.prepare(
            "WITH work AS (
              SELECT session_id,count(*) record_count,min(ts_ms) first_ms,
                CASE WHEN min(model)<>max(model) THEN 'Multiple models' ELSE min(model) END model
              FROM (
                SELECT session_id,ts_ms,model FROM records WHERE is_meta=0
                UNION ALL
                SELECT m.session_id,r.ts_ms,r.model FROM native_record_copies m
                  JOIN records r ON r.uuid=m.record_uuid WHERE r.is_meta=0
              ) GROUP BY session_id
            ), selected AS (
              SELECT s.session_id,s.host,coalesce(s.repo,s.cwd) repo,s.git_branch,
                coalesce(w.record_count,0) record_count,s.has_conflict,
                coalesce(s.started_at_ms,w.first_ms,-9223372036854775808) sort_time,w.model,w.first_ms
              FROM sessions s LEFT JOIN work w ON w.session_id=s.session_id
              WHERE s.kind='user' AND (?2 IS NULL OR s.host=?2)
                AND (?1='' OR instr(lower(coalesce(s.repo,s.cwd,'')),lower(?1))>0
                     OR instr(lower(coalesce(s.git_branch,'')),lower(?1))>0
                     OR instr(lower(s.session_id),lower(?1))>0)
            )
            SELECT session_id,host,repo,git_branch,record_count,has_conflict,sort_time,model,first_ms
            FROM selected WHERE ?3 IS NULL OR sort_time < ?3 OR (sort_time=?3 AND session_id < ?4)
            ORDER BY sort_time DESC,session_id DESC LIMIT 51")?;
        let mut summaries = statement
            .query_map(
                rusqlite::params![
                    search,
                    host,
                    after.map(|a| a.time),
                    after.map(|a| a.id.as_str())
                ],
                |row| {
                    let id: String = row.get(0)?;
                    Ok((
                        SessionSummary {
                            cursor: SessionCursor {
                                time: row.get(6)?,
                                id: id.clone(),
                            },
                            id,
                            host: row.get(1)?,
                            repo: row.get(2)?,
                            branch: row.get(3)?,
                            first_ts: None,
                            record_count: u64::try_from(row.get::<_, i64>(4)?)
                                .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(4, -1))?,
                            has_conflict: row.get(5)?,
                            model: row.get(7)?,
                        },
                        row.get::<_, Option<i64>>(8)?,
                    ))
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        // Derive visible ranges even for sessions not reimported since an upgrade.
        // At most 51 summaries share one read snapshot; preserve sub-ms ordering
        // through the same timestamp helper used by the writer.
        for (summary, first_ms) in &mut summaries {
            summary.first_ts =
                crate::read::first_work_timestamp(&snapshot, &summary.id, *first_ms)?;
        }
        snapshot.commit()?;
        Ok(summaries.into_iter().map(|(summary, _)| summary).collect())
    }
}
