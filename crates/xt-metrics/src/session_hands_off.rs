//! One number per listed session: the median of its M-09 hands-off stretches
//! inside the selected window, with the sample size it came from.
//!
//! Nothing is redefined here. Events, surface timestamp health, the stretch
//! fold and the median are [`crate::hands_off`]'s and [`crate::stats`]'s own,
//! exactly as the Dashboard's hands-off tile and the session detail timeline
//! use them, so a row's median is the median of the very stretches the detail
//! screen draws for that session.
//!
//! The approved row contract:
//!
//! * **Median of this session's own stretches in the window**, with `n`. Medians
//!   are never added or averaged across sessions.
//! * **Health is the surface's.** Timestamp health is judged once over every
//!   session its raw `(host, surface)` has in the whole selected window, then
//!   narrowed to each requested session, as the per-session detail does. A page
//!   never judges a surface from only the sessions it happens to list.
//! * **Unknown stays unknown.** An excluded surface, or an in-window record whose
//!   classification or tool presence leaves a stretch boundary unknown, is
//!   unmeasured with its reason, never a zero.
//! * **No eligible stretches is not zero.** A measured session with no stretch in
//!   the window has `n = 0` and no median.
//!
//! Cost: one pass over the selected window's events for the whole page (the
//! same single load the Dashboard's M-09 makes), then a fold per requested
//! session. No per-row full-window read, transcript, or detail path is used.
use crate::{Error, MetricsDb, Result, Window, hands_off, session, stats};
use serde::Serialize;
use std::collections::BTreeMap;

/// What M-09 can say about one session's stretches inside the selected window.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SessionHandsOff {
    /// No indexed user session owns this identifier.
    Missing,
    /// M-09 states no stretches for this session. `excluded_surface` names the
    /// raw surface whose timestamp health excluded it; `None` means the
    /// session's own in-window records leave a boundary or a segment's tool
    /// presence unknown.
    Unmeasured {
        excluded_surface: Option<hands_off::ExcludedSurface>,
    },
    /// `n` stretches; `median_min` is their median in minutes, absent exactly
    /// when `n` is zero.
    Measured { n: u64, median_min: Option<f64> },
}

impl MetricsDb {
    /// M-09 medians for exactly the requested sessions over one window, in one
    /// snapshot. Duplicate identifiers collapse; an unknown identifier reports
    /// [`SessionHandsOff::Missing`].
    pub fn session_hands_off(
        &self,
        window: Window,
        sessions: &[&str],
    ) -> Result<BTreeMap<String, SessionHandsOff>> {
        let ids: Vec<&str> = {
            let mut unique: Vec<&str> = sessions.to_vec();
            unique.sort_unstable();
            unique.dedup();
            unique
        };
        if ids.len() > session::MAX_SESSIONS {
            return Err(Error::TooManySessions);
        }
        let mut answers = BTreeMap::new();
        if ids.is_empty() {
            return Ok(answers);
        }
        self.read_snapshot(|db| {
            // Which raw surface each requested session belongs to is a metadata
            // fact, read before any measurement so an empty window is never
            // mistaken for an absent session.
            let mut owned = BTreeMap::<String, (String, Option<String>)>::new();
            let mut sql = String::from(session::EXISTS_QUERY);
            sql.push_str(" AND session_id IN (");
            for index in 0..ids.len() {
                if index > 0 {
                    sql.push(',');
                }
                sql.push('?');
                sql.push_str(&(index + 1).to_string());
            }
            sql.push(')');
            let mut statement = db.connection.prepare(&sql)?;
            let mut rows = statement.query(rusqlite::params_from_iter(&ids))?;
            while let Some(row) = rows.next()? {
                owned.insert(row.get(0)?, (row.get(1)?, row.get(2)?));
            }
            drop(rows);
            drop(statement);

            let surfaces = hands_off::load(&db.connection, window)?;
            let mut health = BTreeMap::new();
            for id in ids {
                let Some(key) = owned.get(id) else {
                    answers.insert(id.to_owned(), SessionHandsOff::Missing);
                    continue;
                };
                let sessions = surfaces.get(key);
                let excluded = health
                    .entry(key.clone())
                    .or_insert_with(|| sessions.and_then(|s| hands_off::health(&key.0, &key.1, s)));
                if let Some(excluded) = excluded {
                    answers.insert(
                        id.to_owned(),
                        SessionHandsOff::Unmeasured {
                            excluded_surface: Some(excluded.clone()),
                        },
                    );
                    continue;
                }
                let Some(events) = sessions.and_then(|s| s.get(id)) else {
                    answers.insert(
                        id.to_owned(),
                        SessionHandsOff::Measured {
                            n: 0,
                            median_min: None,
                        },
                    );
                    continue;
                };
                let answer = match hands_off::collect(events)? {
                    None => SessionHandsOff::Unmeasured {
                        excluded_surface: None,
                    },
                    Some(stretches) => {
                        let mut durations: Vec<u64> =
                            stretches.iter().map(|s| s.duration_ms()).collect();
                        let n = durations.len() as u64;
                        SessionHandsOff::Measured {
                            n,
                            median_min: stats::median_p90(&mut durations)
                                .map(|(median, _)| median / 60000.0),
                        }
                    }
                };
                answers.insert(id.to_owned(), answer);
            }
            Ok(())
        })?;
        Ok(answers)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requested_sessions_are_bounded() {
        let db = xt_fixtures::TempDb::empty().unwrap();
        let metrics = MetricsDb::open(db.path()).unwrap();
        let window = Window::new(0, 1000).unwrap();
        let many: Vec<String> = (0..=session::MAX_SESSIONS)
            .map(|i| format!("s{i}"))
            .collect();
        let ids: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(metrics.session_hands_off(window, &ids).is_err());
        let answers = metrics.session_hands_off(window, &ids[1..]).unwrap();
        assert!(answers.values().all(|a| *a == SessionHandsOff::Missing));
        assert!(metrics.session_hands_off(window, &[]).unwrap().is_empty());
    }
}
