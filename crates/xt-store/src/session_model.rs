//! Which model a session used most. One rule, read by the Sessions list (over
//! the whole session) and by the Dashboard's effort-by-model split (over the
//! selected window), so the same work never names two different models.
//!
//! A use is one selected response: one `v_response_usage` row (the M-04
//! response selection the cost report prices) that names a model. The model
//! with the most such responses wins, then the one with the most output
//! tokens, then the first name. Only when none of the work in scope has a
//! selected response that names a model (Cursor sessions without hook
//! counters) are the assistant records that name a model and carry no usage
//! counted instead, one use each, ties again going to the first name.
use std::collections::BTreeMap;

/// The model name a record states, as the cost report reads it: absent, empty
/// or whitespace only (Rust's Unicode `trim`) is no model. Every model use
/// here and every priced response go through this one rule.
pub fn named_model(model: Option<&str>) -> Option<&str> {
    model.filter(|name| !name.trim().is_empty())
}

/// The assistant work records that name a model but carry no usage
/// observation: the uses counted only when no selected response names a model.
macro_rules! unmetered_records_sql {
    () => {
        concat!(
            "SELECT r.session_id,r.model,r.ts FROM ",
            assistant_work_rows_sql!(),
            " AND u.uuid IS NULL AND r.model IS NOT NULL"
        )
    };
}

/// [`unmetered_records_sql`] for one event window: `?1`/`?2` are the window's
/// candidate millisecond bounds. Columns: session, model, exact timestamp, so
/// a caller applies the same exact-instant membership its responses use.
pub const UNMETERED_RECORDS_IN_WINDOW_SQL: &str =
    concat!(unmetered_records_sql!(), " AND r.ts_ms>=?1 AND r.ts_ms<?2");

/// The named sessions' model uses in one pass over their assistant work
/// records, grouped by session, model and whether the uses are selected
/// responses (`1`, with their output tokens) or records without usage (`0`).
/// The responses are `v_response_usage` for just these sessions: the query is
/// built from the very rules that view is built from ([`crate::views`]: the
/// work rows, the keyed-response test with its whitespace set, and the
/// latest-snapshot selection), so the two cannot differ, without paying for
/// the view's classification joins on every record. `{list}` is the `IN` list.
const WHOLE_SESSION_SQL: &str = concat!(
    "WITH ",
    whitespace_sql!(),
    ", t AS (
    SELECT *, ",
    response_keyed_sql!(),
    " AS response_keyed FROM (
      SELECT r.session_id,r.uuid,r.ts,r.api_message_id,r.request_id,s.host,r.model,
        u.output_tokens,u.uuid IS NOT NULL AS metered
      FROM ",
    assistant_work_rows_sql!(),
    " AND r.session_id IN {list}
    ) CROSS JOIN whitespace
)
SELECT t.session_id,t.model,t.metered,count(*),sum(coalesce(t.output_tokens,0)) FROM t
WHERE t.model IS NOT NULL AND (t.metered=0 OR ",
    response_selected_sql!(),
    ") GROUP BY t.session_id,t.model,t.metered"
);

#[derive(Clone, Copy, Debug, Default)]
struct Use {
    responses: u64,
    output_tokens: u64,
    unmetered: u64,
}

/// The uses of each model in some piece of one session's work.
#[derive(Clone, Debug, Default)]
pub struct ModelUses {
    models: BTreeMap<String, Use>,
}

/// The model a session used most, and how many other models it used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MostUsedModel {
    pub model: String,
    /// Distinct other model names the same work recorded, metered or not.
    pub other_models: u64,
}

impl ModelUses {
    /// Selected responses that name `model`, with their output tokens (an
    /// unknown count adds nothing). A blank name ([`named_model`]) is no model
    /// and adds nothing.
    pub fn add_responses(&mut self, model: &str, responses: u64, output_tokens: u64) {
        if named_model(Some(model)).is_none() {
            return;
        }
        let entry = self.models.entry(model.to_owned()).or_default();
        entry.responses = entry.responses.saturating_add(responses);
        entry.output_tokens = entry.output_tokens.saturating_add(output_tokens);
    }

    /// Assistant records that name `model` and carry no usage observation. A
    /// blank name ([`named_model`]) is no model and adds nothing.
    pub fn add_unmetered_records(&mut self, model: &str, records: u64) {
        if named_model(Some(model)).is_none() {
            return;
        }
        let entry = self.models.entry(model.to_owned()).or_default();
        entry.unmetered = entry.unmetered.saturating_add(records);
    }

    /// The most-used model under the rule this module states, or `None` when
    /// nothing named a model.
    pub fn most_used(&self) -> Option<MostUsedModel> {
        let metered = self.models.values().any(|use_| use_.responses > 0);
        let rank = |use_: &Use| {
            if metered {
                (use_.responses, use_.output_tokens)
            } else {
                (use_.unmetered, 0)
            }
        };
        self.models
            .iter()
            .max_by(|(a_name, a), (b_name, b)| {
                rank(a).cmp(&rank(b)).then_with(|| b_name.cmp(a_name))
            })
            .map(|(model, _)| MostUsedModel {
                model: model.clone(),
                other_models: self.models.len() as u64 - 1,
            })
    }
}

/// Each named session's model uses over its whole history, against a
/// caller-owned read snapshot. Only the session's own records count: records a
/// fork shares with the session it was forked from are the owner's work, as in
/// every measurement. Sessions with no use are absent.
pub fn whole_sessions(
    connection: &rusqlite::Connection,
    ids: &[&str],
) -> crate::Result<BTreeMap<String, ModelUses>> {
    let mut uses = BTreeMap::<String, ModelUses>::new();
    if ids.is_empty() {
        return Ok(uses);
    }
    let mut list = String::from("(");
    for index in 0..ids.len() {
        if index > 0 {
            list.push(',');
        }
        list.push('?');
        list.push_str(&(index + 1).to_string());
    }
    list.push(')');
    let count = |row: &rusqlite::Row<'_>, column| -> rusqlite::Result<u64> {
        Ok(u64::try_from(row.get::<_, Option<i64>>(column)?.unwrap_or(0)).unwrap_or(0))
    };
    let mut statement = connection.prepare(&WHOLE_SESSION_SQL.replace("{list}", &list))?;
    let mut rows = statement.query(rusqlite::params_from_iter(ids))?;
    while let Some(row) = rows.next()? {
        let model: String = row.get(1)?;
        let entry = uses.entry(row.get(0)?).or_default();
        if row.get::<_, bool>(2)? {
            entry.add_responses(&model, count(row, 3)?, count(row, 4)?);
        } else {
            entry.add_unmetered_records(&model, count(row, 3)?);
        }
    }
    drop(rows);
    uses.retain(|_, session| !session.models.is_empty());
    Ok(uses)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn most(uses: &ModelUses) -> Option<(String, u64)> {
        uses.most_used().map(|m| (m.model, m.other_models))
    }
    fn named(model: &str, others: u64) -> Option<(String, u64)> {
        Some((model.to_owned(), others))
    }

    #[test]
    fn more_responses_win_then_output_then_first_name() {
        let mut uses = ModelUses::default();
        uses.add_responses("opus", 200, 1);
        uses.add_responses("haiku", 1, 10_000);
        assert_eq!(most(&uses), named("opus", 1));

        let mut tied = ModelUses::default();
        tied.add_responses("b", 2, 5);
        tied.add_responses("a", 2, 4);
        assert_eq!(most(&tied), named("b", 1));

        let mut even = ModelUses::default();
        even.add_responses("b", 2, 5);
        even.add_responses("a", 2, 5);
        assert_eq!(most(&even), named("a", 1));
    }

    #[test]
    fn unmetered_records_count_only_when_no_response_names_a_model() {
        let mut uses = ModelUses::default();
        uses.add_unmetered_records("many-records", 50);
        uses.add_responses("one-response", 1, 0);
        assert_eq!(most(&uses), named("one-response", 1));

        let mut unmetered = ModelUses::default();
        unmetered.add_unmetered_records("b", 3);
        unmetered.add_unmetered_records("a", 3);
        unmetered.add_unmetered_records("c", 1);
        assert_eq!(most(&unmetered), named("a", 2));

        assert_eq!(ModelUses::default().most_used(), None);
    }

    #[test]
    fn a_blank_model_name_is_no_model_as_pricing_reads_it() {
        assert_eq!(named_model(Some(" \u{3000}\t")), None);
        assert_eq!(named_model(Some(" opus ")), Some(" opus "));
        let mut uses = ModelUses::default();
        uses.add_responses("  ", 9, 9);
        uses.add_unmetered_records("\u{a0}", 9);
        uses.add_responses("opus", 1, 0);
        assert_eq!(most(&uses), named("opus", 0));
    }
}
