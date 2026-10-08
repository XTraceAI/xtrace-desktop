//! Totals over every indexed session matching the store's list filter.
//! Membership is independent of the event window, just like the Sessions table.
use crate::{MetricsDb, Result, SessionWindow, Window};
use serde::Serialize;
use xt_store::session_list::{ChildCheck, SessionFilter};

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct SessionListSummary {
    pub main_sessions: u64,
    pub sub_sessions: u64,
    pub checking_sessions: u64,
    pub unlinked_sub_sessions: u64,
    pub human_messages: Option<u64>,
    pub messages_per_main_session: Option<f64>,
    pub agent_ms: Option<u64>,
    pub sessions_with_prs: u64,
}

impl MetricsDb {
    /// Read all matching pages and reuse each session's M-02 and M-05 answer.
    /// Counts describe indexed matches, including sub-sessions the table hides
    /// until a creator is verified. A pending classification never divides the
    /// input total by a main-session count that could still change.
    pub fn sessions_summary(
        &self,
        window: Window,
        filter: &SessionFilter<'_>,
    ) -> Result<SessionListSummary> {
        self.read_snapshot(|db| {
            let mut summary = SessionListSummary {
                human_messages: Some(0),
                agent_ms: Some(0),
                ..Default::default()
            };
            let mut after = None;
            loop {
                let rows = db.sessions_page_filtered(filter, after.as_ref())?;
                if rows.is_empty() {
                    break;
                }
                let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
                let context = db.session_context(&ids)?;
                for row in context {
                    if row.known_child || row.parent.is_some() || row.check == ChildCheck::Child {
                        summary.sub_sessions += 1;
                        if row.parent.is_none() {
                            summary.unlinked_sub_sessions += 1;
                        }
                    } else if row.check == ChildCheck::Checking {
                        summary.checking_sessions += 1;
                    } else {
                        summary.main_sessions += 1;
                    }
                }
                summary.sessions_with_prs +=
                    rows.iter().filter(|row| !row.pr_links.is_empty()).count() as u64;
                for measured in db.session_windows(window, &ids)?.into_values() {
                    match measured {
                        SessionWindow::Indexed {
                            human_messages,
                            agent_ms,
                            ..
                        } => {
                            summary.human_messages =
                                crate::counts::add(summary.human_messages, human_messages)?;
                            summary.agent_ms =
                                crate::counts::add(summary.agent_ms, Some(agent_ms))?;
                        }
                        SessionWindow::Missing => {
                            summary.human_messages = None;
                            summary.agent_ms = None;
                        }
                    }
                }
                if rows.len() < 51 {
                    break;
                }
                after = rows.last().map(|row| row.cursor.clone());
            }
            summary.messages_per_main_session = summary
                .human_messages
                .filter(|_| summary.main_sessions > 0 && summary.checking_sessions == 0)
                .map(|messages| messages as f64 / summary.main_sessions as f64);
            Ok(summary)
        })
    }
}
