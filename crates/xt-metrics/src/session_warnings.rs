//! M-21 session warnings: simple rules that say a session probably went
//! wrong somewhere. No model and no score: a warning fires when one rule
//! matches, and each warning stands on its own with a plain-English reason.
//!
//! [`decide`] is the one function that decides a session's warnings. It reads
//! only the evidence handed to it; [`MetricsDb::session_warnings`] gathers that
//! evidence from the store and calls it once per session, and anything that
//! later shows a warning reads its result rather than working one out again.
//!
//! The five rules, and the existing answers each one reads:
//!
//! * **Correction**: a message the stored M-02 classification calls a
//!   person's, sent after an agent reply, holds a correction ([`correction`]).
//! * **Interruption**: an input the store flagged as Claude Code's
//!   `[Request interrupted by user…]` marker (`records.is_interrupted`), an
//!   input with a stored proof that Codex wrote its `<turn_aborted>` note
//!   (`tool_sent_inputs`), or a tool call whose saved result says the person
//!   declined or interrupted it. A stopped call and Claude Code's marker for
//!   it are one interruption, not two.
//! * **Circling**: an M-09 stretch judged circling by M-20's own measurement
//!   and thresholds, over every call except the expected repeats
//!   [`expected_repeat`] names (see [`crate::repeats`]'s call scope).
//! * **Repeated prompt**: the same person's message, compared lowercased with
//!   whitespace collapsed, sent two or more times in the session; short
//!   messages are not compared ([`comparable_prompt`]).
//! * **Repeated failure**: two or more calls with the same stored M-20
//!   comparison key whose saved result says they failed.
//!
//! A person's message that opens with a tag (`<ci-monitor-event …>`,
//! `<task-notification>`) is text a tool wrote ([`tool_written`]): neither
//! text rule reads it, and it does not end the agent's reply.
//!
//! Sidechain records (an agent's own sub-conversation inside the session) are
//! left out of the rules that read messages and results: they are not the
//! person's conversation. Circling is the exception on purpose. It is M-20's
//! answer over M-09's stretches, which count sidechain records and calls as
//! the Dashboard does, so a stretch and its repeats are the same here as there.
//! Leaving sidechains out only here would give "is this stretch circling" a
//! second answer.
//!
//! An unknown is never a zero. Each rule says whether it read everything it
//! needed ([`CheckState::Complete`]) or which inputs it could not read. A
//! warning found among the inputs it could read is still a warning; an
//! incomplete rule with no warning says nothing about the session.

use crate::{
    MetricsDb, Result, Window, hands_off,
    repeats::{
        CallScope, RepeatDensity, RepeatThresholds, SessionRepeats, StoredCalls, UnknownRepeats,
    },
    span_detail::{self, PersonText},
};
use jiff::{Timestamp, tz::TimeZone};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use xt_store::{
    Host,
    timestamp::{self, InstantKey},
    tool_result::{self, Outcome, Wording},
    tool_sent::ToolSentKind,
};

/// One session's records inside the window, with the stored facts every rule
/// reads: who sent each one (M-02), whether only part of a person's record is
/// theirs, the stored interruption flag, the stated tool-result presence, the
/// record it follows in its source, and whether a stored proof says Codex
/// wrote it as its interrupted-turn note (`?4`).
pub(crate) const RECORD_QUERY: &str = "SELECT v.uuid,v.ts,v.ts_ms,v.type,v.is_sidechain,
       v.human_is_eligible,v.human_text_len IS NOT v.text_len,v.is_interrupted,
       v.is_tool_result_carrier,v.parent_uuid,
       EXISTS(SELECT 1 FROM tool_sent_inputs t WHERE t.record_uuid=v.uuid
           AND t.session_id=v.session_id AND t.evidence_kind=?4)
     FROM v_session_events v
     WHERE v.session_id=?1 AND v.ts_ms>=?2 AND v.ts_ms<?3";

/// The saved facts of every tool result in the session (migration 27): the
/// result's record, the call it names, its flag and wording kind, the facts'
/// version and the session's host. A conflicted row or one on a conflicted
/// record reads as unknown.
pub(crate) const RESULT_QUERY: &str = "SELECT t.record_uuid,t.call_id,t.is_error,t.flag_stated,
       t.wording,t.fact_version,t.conflicted OR r.has_conflict,s.host
     FROM tool_results t JOIN records r ON r.uuid=t.record_uuid AND r.session_id=t.session_id
     JOIN sessions s ON s.session_id=t.session_id
     WHERE t.session_id=?1";

/// Every call in the session whose identifier is known and uncontested.
pub(crate) const CALL_QUERY: &str = "SELECT uuid,block_index,call_id FROM tool_uses
     WHERE session_id=?1 AND uuid IS NOT NULL AND call_id IS NOT NULL AND call_id_conflict=0";

/// Words that, on their own, mark a person's message as a correction.
pub const CORRECTION_WORDS: [&str; 3] = ["wrong", "stop", "again"];
/// "no" counts only as the first word of a message of at most this many
/// words ("No, use the other file"), never inside a longer one ("there is
/// no test for it yet").
pub const NO_MAX_WORDS: usize = 8;
/// Two words that, in this order, mark a correction.
pub const CORRECTION_PHRASE: [&str; 2] = ["i", "said"];
/// "still" counts only with one of these within [`STILL_REACH`] words of it,
/// before or after: "still not working" is a correction, "I still need an
/// export button" is not.
pub const STILL_NEGATIVES: [&str; 9] = [
    "not",
    "broken",
    "fails",
    "error",
    "same",
    "wrong",
    "doesn't",
    "happening",
    "see",
];
pub const STILL_REACH: usize = 5;
/// A repeated prompt or a repeated failure needs at least this many.
pub const REPEAT_AT_LEAST: usize = 2;
/// A prompt of fewer words ("yes", "go on") is not compared with the others.
pub const PROMPT_MIN_WORDS: usize = 4;

/// Tools that track a task list: each call is the list's next state, so
/// calling them often is their job, not going in circles.
pub const TASK_LIST_TOOLS: [&str; 7] = [
    "TodoWrite",
    "TaskCreate",
    "TaskUpdate",
    "TaskList",
    "TaskGet",
    "update_plan",
    "mcp__ccd_turn__update_plan",
];
/// Tools that wait or poll for something to finish.
pub const WAITING_TOOLS: [&str; 8] = [
    "ScheduleWakeup",
    "Monitor",
    "sleep",
    "wait",
    "wait_agent",
    "AwaitShell",
    "BashOutput",
    "TaskOutput",
];

/// Tools a subagent calls to hand its report back to the agent that started
/// it: one call per subagent, so a session that runs many of them calls it
/// often.
pub const HANDBACK_TOOLS: [&str; 1] = ["SubagentHandback"];

/// M-21's circling exclusion: whether a call is an expected repeat, which
/// the circling warning does not compare. Read from the stored tool name
/// only; no argument is read. A task-list tool ([`TASK_LIST_TOOLS`]), a
/// waiting tool ([`WAITING_TOOLS`]), a handback tool ([`HANDBACK_TOOLS`]),
/// or a screenshot tool: its stored name contains `screenshot`.
pub fn expected_repeat(name: &str) -> bool {
    TASK_LIST_TOOLS.contains(&name)
        || WAITING_TOOLS.contains(&name)
        || HANDBACK_TOOLS.contains(&name)
        || name.to_ascii_lowercase().contains("screenshot")
}

/// The five rules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    Correction,
    Interruption,
    Circling,
    RepeatedPrompt,
    RepeatedFailure,
}

impl WarningKind {
    pub const ALL: [Self; 5] = [
        Self::Correction,
        Self::Interruption,
        Self::Circling,
        Self::RepeatedPrompt,
        Self::RepeatedFailure,
    ];
}

/// One warning: which rule matched, where, and why in plain words.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionWarning {
    pub kind: WarningKind,
    pub session_id: String,
    /// The moment it points at, on the stored millisecond projection: the
    /// message, the interruption, the stretch's start, the first repeat.
    pub at_ms: i64,
    /// The saved records it points at, earliest first.
    pub record_uuids: Vec<String>,
    /// One short sentence. A correction quotes only the matched words; no
    /// other message text is ever part of it.
    pub reason: String,
}

/// Inputs a rule needed and could not read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "missing", rename_all = "snake_case")]
pub enum Missing {
    /// A person's messages whose words neither content retention nor the
    /// index's short preview kept.
    TextNotKept { messages: u64 },
    /// A person's messages of which the index kept only the first
    /// [`span_detail::MAX_PROMPT_CHARS`] characters: the correction rule found
    /// nothing in that part, or the repeated-prompt rule could not compare
    /// the whole message.
    TextCut { messages: u64 },
    /// A person's messages only part of which is theirs (a Codex question
    /// reply or image wrapper); nothing locates their part.
    WrappedText { messages: u64 },
    /// Inputs whose M-02 classification is unknown: any of them may be a
    /// person's message.
    Unclassified { inputs: u64 },
    /// Inputs the store could not flag as an interruption or not.
    InterruptionUnknown { inputs: u64 },
    /// Records carrying tool results for which the index saved no outcome:
    /// indexed before outcomes were kept, or their content was never seen.
    OutcomeNotSaved { records: u64 },
    /// Tool results whose saved outcome is unknown: the host does not state
    /// one (Codex, Cursor), two observations disagreed, or a later build
    /// wrote them.
    OutcomeUnknown { results: u64 },
    /// Failed results naming a call the session's saved calls do not
    /// identify.
    UnmatchedResults { results: u64 },
    /// Failed calls whose stored comparison key cannot be compared.
    FailedCallKey { reason: UnknownRepeats, calls: u64 },
    /// M-20 states no stretches for this session: its surface's clock is
    /// excluded, or a classification it needs is unknown.
    CirclingUnmeasured,
    /// M-20 could not count the calls of some stretches.
    RepeatsUnknown {
        reason: UnknownRepeats,
        stretches: u64,
    },
}

/// Whether a rule read every input it needed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum CheckState {
    /// Everything was read: no warning of this kind means none happened.
    Complete,
    /// Some inputs could not be read. Warnings found are real; their absence
    /// proves nothing.
    Incomplete { missing: Vec<Missing> },
}

impl CheckState {
    fn of(missing: Vec<Missing>) -> Self {
        if missing.is_empty() {
            Self::Complete
        } else {
            Self::Incomplete { missing }
        }
    }
    pub fn is_complete(&self) -> bool {
        matches!(self, Self::Complete)
    }
}

/// Each rule's coverage for one session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WarningChecks {
    pub correction: CheckState,
    pub interruption: CheckState,
    pub circling: CheckState,
    pub repeated_prompt: CheckState,
    pub repeated_failure: CheckState,
}

impl WarningChecks {
    pub fn get(&self, kind: WarningKind) -> &CheckState {
        match kind {
            WarningKind::Correction => &self.correction,
            WarningKind::Interruption => &self.interruption,
            WarningKind::Circling => &self.circling,
            WarningKind::RepeatedPrompt => &self.repeated_prompt,
            WarningKind::RepeatedFailure => &self.repeated_failure,
        }
    }
}

/// M-21 for one session: its warnings, earliest first, and each rule's
/// coverage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SessionWarnings {
    pub session_id: String,
    pub warnings: Vec<SessionWarning>,
    pub checks: WarningChecks,
    /// Messages the stored classification calls a person's, outside
    /// sidechains, and how many of them the text rules read in full.
    pub person_messages: u64,
    pub person_messages_read: u64,
    /// Tool results in the window, and how many have a saved, known outcome.
    pub tool_results: u64,
    pub tool_results_known: u64,
}

/// One record as the rules read it.
pub(crate) struct EvidenceRecord {
    pub(crate) uuid: String,
    pub(crate) at_ms: i64,
    pub(crate) agent: bool,
    pub(crate) sidechain: bool,
    /// M-02: `Some(true)` a person's message, `None` unknown.
    pub(crate) person: Option<bool>,
    /// A person's message's words; `None` for every other record.
    pub(crate) words: Option<PersonText>,
    /// The store says the person interrupted the agent here: Claude Code's
    /// marker, or a stored proof of Codex's interrupted-turn note. `None`
    /// unknown.
    pub(crate) interrupted: Option<bool>,
}

/// The call a result answers, from its stored metadata.
pub(crate) struct CallFacts {
    /// The call's stored tool name.
    pub(crate) name: String,
    /// The call's M-20 comparison key, or why it cannot be compared.
    pub(crate) key: std::result::Result<(u32, String), UnknownRepeats>,
    /// The key says the call named no identifying argument, so it only
    /// identifies the tool: a reason then names the tool, not "the same call".
    pub(crate) tool_only: bool,
}

/// One failed, declined or interrupted call, from its saved result.
pub(crate) struct ToolOutcome {
    /// The record carrying the result.
    pub(crate) result_uuid: String,
    pub(crate) at_ms: i64,
    /// [`Outcome::Failed`], [`Outcome::Declined`] or [`Outcome::Interrupted`].
    pub(crate) ended: Outcome,
    /// The call it answers, when exactly one saved call has its identifier.
    pub(crate) call: Option<CallFacts>,
}

/// Everything [`decide`] reads about one session.
pub(crate) struct Evidence {
    pub(crate) session_id: String,
    /// Chronological: exact instant, then source order (a record after the
    /// one it follows), then UUID.
    pub(crate) records: Vec<EvidenceRecord>,
    /// Failed, declined and interrupted calls only.
    pub(crate) outcomes: Vec<ToolOutcome>,
    pub(crate) tool_results: u64,
    pub(crate) tool_results_known: u64,
    /// Result-carrying records with no saved outcome.
    pub(crate) outcome_not_saved: u64,
    pub(crate) outcome_unknown: u64,
    /// M-20 over the session's calls, expected repeats left out.
    pub(crate) repeats: SessionRepeats,
    /// The calls, by `(record, block)`, whose M-20 key names the tool only,
    /// among the most repeated call of each circling stretch.
    pub(crate) tool_only_calls: BTreeSet<(String, u64)>,
}

/// The words of a message as the text rules compare them: lowercase, curly
/// apostrophes made straight, split on anything that is not a letter, digit,
/// apostrophe or hyphen.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .replace('\u{2019}', "'")
        .split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '-'))
        .map(|word| word.trim_matches(|c| c == '\'' || c == '-'))
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The first correction in a message, as the matched words, or `None`.
pub fn correction(text: &str) -> Option<String> {
    find_correction(text, true)
}

/// [`correction`] over a message's words. `whole` says they are all of the
/// message; for only its start, the "no" rule cannot run, because how many
/// words the whole message has is not known.
fn find_correction(text: &str, whole: bool) -> Option<String> {
    let words = words(text);
    if whole && words.first().map(String::as_str) == Some("no") && words.len() <= NO_MAX_WORDS {
        return Some("no".to_owned());
    }
    for (index, word) in words.iter().enumerate() {
        if CORRECTION_WORDS.contains(&word.as_str()) {
            return Some(word.clone());
        }
        if word == CORRECTION_PHRASE[0]
            && words.get(index + 1).map(String::as_str) == Some(CORRECTION_PHRASE[1])
        {
            return Some(CORRECTION_PHRASE.join(" "));
        }
        if word == "still" {
            let low = index.saturating_sub(STILL_REACH);
            let high = (index + STILL_REACH).min(words.len() - 1);
            // The nearest negative word, after before before on a tie.
            let near = (1..=STILL_REACH)
                .flat_map(|distance| [index.checked_add(distance), index.checked_sub(distance)])
                .flatten()
                .filter(|other| (low..=high).contains(other))
                .find(|other| STILL_NEGATIVES.contains(&words[*other].as_str()));
            if let Some(other) = near {
                return Some(words[index.min(other)..=index.max(other)].join(" "));
            }
        }
    }
    None
}

/// Whether a person's message opens with a tag such as
/// `<ci-monitor-event …>` or `<task-notification>`: text a tool or an agent
/// wrote, whatever the classification says about who sent it. Decided once
/// per message; neither text rule reads such a message.
pub fn tool_written(text: &str) -> bool {
    let mut chars = text.trim_start().chars();
    chars.next() == Some('<') && chars.next().is_some_and(|c| c.is_ascii_alphabetic())
}

/// How a prompt is compared with the session's other prompts, or `None` when
/// it is not compared: fewer than [`PROMPT_MIN_WORDS`] words.
pub fn comparable_prompt(text: &str) -> Option<String> {
    if words(text).len() < PROMPT_MIN_WORDS {
        return None;
    }
    Some(
        text.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase(),
    )
}

/// The start of a cut message without its last word, which the cut may have
/// split ("stop" of "stopwatch").
fn whole_words_of_start(text: &str) -> &str {
    text.trim_end()
        .rsplit_once(char::is_whitespace)
        .map_or("", |(head, _)| head)
}

fn times_word(count: usize) -> String {
    match count {
        2 => "twice".to_owned(),
        n => format!("{n} times"),
    }
}

/// How a reason names a group of calls that share one M-20 key: "the same
/// Bash call", or only the tool when its key names the tool only.
fn call_words(tool: &str, tool_only: bool) -> String {
    if tool_only {
        tool.to_owned()
    } else {
        format!("the same {tool} call")
    }
}

/// Local clock times, the date added to any not on the first one's day.
fn clock_times(moments: &[i64], zone: &TimeZone) -> String {
    let zoned: Vec<_> = moments
        .iter()
        .filter_map(|ms| Timestamp::from_millisecond(*ms).ok())
        .map(|at| at.to_zoned(zone.clone()))
        .collect();
    let first_day = zoned.first().map(|at| at.date());
    zoned
        .iter()
        .map(|at| {
            if Some(at.date()) == first_day {
                at.strftime("%H:%M").to_string()
            } else {
                at.strftime("%b %-d %H:%M").to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The interruption warning the next record may still belong to.
struct OpenStop {
    index: usize,
    at_ms: i64,
    /// From a stopped call's result rather than from a marker.
    from_call: bool,
}

/// Decide one session's warnings from its evidence. This is M-21: every
/// warning, and every statement about what could not be checked, comes from
/// here.
pub(crate) fn decide(evidence: &Evidence, zone: &TimeZone) -> SessionWarnings {
    let session_id = &evidence.session_id;
    let mut warnings: Vec<SessionWarning> = Vec::new();
    let warn = |kind, at_ms, record_uuids: Vec<String>, reason: String| SessionWarning {
        kind,
        session_id: session_id.clone(),
        at_ms,
        record_uuids,
        reason,
    };
    let mut stops = BTreeMap::<&str, Vec<&ToolOutcome>>::new();
    for outcome in evidence
        .outcomes
        .iter()
        .filter(|o| matches!(o.ended, Outcome::Declined | Outcome::Interrupted))
    {
        stops
            .entry(outcome.result_uuid.as_str())
            .or_default()
            .push(outcome);
    }

    // Corrections, repeated prompts and interruptions walk the person's own
    // conversation once.
    let mut replied = false;
    // A stopped call and Claude Code's marker for it are one interruption:
    // the marker after the call, until an agent record or a person's message,
    // or a marker and a call at the very same moment in either order.
    let mut open: Option<OpenStop> = None;
    let mut unclassified = 0;
    let (mut correction_not_kept, mut correction_cut, mut correction_wrapped) = (0, 0, 0);
    let (mut prompt_not_kept, mut prompt_cut, mut prompt_wrapped) = (0, 0, 0);
    let mut interruption_unknown = 0;
    let mut prompts = BTreeMap::<String, Vec<&EvidenceRecord>>::new();
    let (mut person_messages, mut person_messages_read) = (0, 0);
    for record in evidence.records.iter().filter(|r| !r.sidechain) {
        if record.agent {
            replied = true;
            open = None;
            continue;
        }
        for outcome in stops.get(record.uuid.as_str()).into_iter().flatten() {
            let tool = outcome.call.as_ref().map_or("a tool", |c| c.name.as_str());
            let verb = match outcome.ended {
                Outcome::Declined => "declined",
                _ => "interrupted",
            };
            let reason = format!(
                "you {verb} a call to {tool} ({})",
                clock_times(&[outcome.at_ms], zone)
            );
            match &open {
                // The marker came first at the same moment: the same
                // interruption, now named by the call.
                Some(stop) if !stop.from_call && stop.at_ms == outcome.at_ms => {
                    let warning = &mut warnings[stop.index];
                    warning.record_uuids.push(record.uuid.clone());
                    warning.reason = reason;
                    open = None;
                }
                _ => {
                    warnings.push(warn(
                        WarningKind::Interruption,
                        outcome.at_ms,
                        vec![record.uuid.clone()],
                        reason,
                    ));
                    open = Some(OpenStop {
                        index: warnings.len() - 1,
                        at_ms: outcome.at_ms,
                        from_call: true,
                    });
                }
            }
        }
        match record.interrupted {
            Some(true) => match open.take() {
                Some(stop) if stop.from_call => {
                    warnings[stop.index].record_uuids.push(record.uuid.clone())
                }
                _ => {
                    warnings.push(warn(
                        WarningKind::Interruption,
                        record.at_ms,
                        vec![record.uuid.clone()],
                        format!(
                            "you interrupted the agent ({})",
                            clock_times(&[record.at_ms], zone)
                        ),
                    ));
                    open = Some(OpenStop {
                        index: warnings.len() - 1,
                        at_ms: record.at_ms,
                        from_call: false,
                    });
                }
            },
            Some(false) => {}
            None => interruption_unknown += 1,
        }
        match record.person {
            Some(true) => {}
            Some(false) => continue,
            None => {
                unclassified += 1;
                continue;
            }
        }
        person_messages += 1;
        // A tool-written message: like an automatic input, it neither counts
        // for the text rules nor ends the agent's reply.
        if let Some(PersonText::Whole(text) | PersonText::Start(text)) = &record.words
            && tool_written(text)
        {
            person_messages_read += u64::from(matches!(record.words, Some(PersonText::Whole(_))));
            continue;
        }
        open = None;
        let after_reply = std::mem::replace(&mut replied, false);
        let mut corrected = |phrase: String| {
            warnings.push(warn(
                WarningKind::Correction,
                record.at_ms,
                vec![record.uuid.clone()],
                format!(
                    "you wrote \"{phrase}\" after a reply ({})",
                    clock_times(&[record.at_ms], zone)
                ),
            ))
        };
        match &record.words {
            Some(PersonText::Whole(text)) => {
                person_messages_read += 1;
                if after_reply && let Some(phrase) = correction(text) {
                    corrected(phrase);
                }
                if let Some(key) = comparable_prompt(text) {
                    prompts.entry(key).or_default().push(record);
                }
            }
            Some(PersonText::Start(text)) => {
                if after_reply {
                    match find_correction(whole_words_of_start(text), false) {
                        Some(phrase) => corrected(phrase),
                        None => correction_cut += 1,
                    }
                }
                prompt_cut += 1;
            }
            Some(PersonText::Wrapped) => {
                correction_wrapped += u64::from(after_reply);
                prompt_wrapped += 1;
            }
            Some(PersonText::NotStored) | None => {
                correction_not_kept += u64::from(after_reply);
                prompt_not_kept += 1;
            }
        }
    }
    for sent in prompts
        .values()
        .filter(|sent| sent.len() >= REPEAT_AT_LEAST)
    {
        let moments: Vec<_> = sent.iter().map(|r| r.at_ms).collect();
        warnings.push(warn(
            WarningKind::RepeatedPrompt,
            sent[1].at_ms,
            sent.iter().map(|r| r.uuid.clone()).collect(),
            format!(
                "same prompt sent {} ({})",
                times_word(sent.len()),
                clock_times(&moments, zone)
            ),
        ));
    }
    let text_missing = |not_kept: u64, cut: u64, wrapped: u64| {
        let mut missing = Vec::new();
        if not_kept > 0 {
            missing.push(Missing::TextNotKept { messages: not_kept });
        }
        if cut > 0 {
            missing.push(Missing::TextCut { messages: cut });
        }
        if wrapped > 0 {
            missing.push(Missing::WrappedText { messages: wrapped });
        }
        if unclassified > 0 {
            missing.push(Missing::Unclassified {
                inputs: unclassified,
            });
        }
        missing
    };

    // Failed calls, grouped by the call's comparison key.
    let mut failures = BTreeMap::<&(u32, String), Vec<(&ToolOutcome, &CallFacts)>>::new();
    let mut keyless = BTreeMap::<UnknownRepeats, u64>::new();
    let mut unmatched = 0;
    for outcome in evidence
        .outcomes
        .iter()
        .filter(|o| o.ended == Outcome::Failed)
    {
        match &outcome.call {
            None => unmatched += 1,
            Some(call) => match &call.key {
                Ok(key) => failures.entry(key).or_default().push((outcome, call)),
                Err(reason) => *keyless.entry(*reason).or_default() += 1,
            },
        }
    }
    for failed in failures.values().filter(|f| f.len() >= REPEAT_AT_LEAST) {
        let moments: Vec<_> = failed.iter().map(|(o, _)| o.at_ms).collect();
        let (_, call) = failed[0];
        warnings.push(warn(
            WarningKind::RepeatedFailure,
            failed[1].0.at_ms,
            failed.iter().map(|(o, _)| o.result_uuid.clone()).collect(),
            format!(
                "{} failed {} ({})",
                call_words(&call.name, call.tool_only),
                times_word(failed.len()),
                clock_times(&moments, zone)
            ),
        ));
    }
    let mut outcome_missing = Vec::new();
    if evidence.outcome_not_saved > 0 {
        outcome_missing.push(Missing::OutcomeNotSaved {
            records: evidence.outcome_not_saved,
        });
    }
    if evidence.outcome_unknown > 0 {
        outcome_missing.push(Missing::OutcomeUnknown {
            results: evidence.outcome_unknown,
        });
    }
    let mut failure_missing = outcome_missing.clone();
    if unmatched > 0 {
        failure_missing.push(Missing::UnmatchedResults { results: unmatched });
    }
    failure_missing.extend(
        keyless
            .into_iter()
            .map(|(reason, calls)| Missing::FailedCallKey { reason, calls }),
    );
    // A stopped call whose call is unknown was still found: only its tool's
    // name is missing, which the reason words without.
    let mut interruption_missing = Vec::new();
    if interruption_unknown > 0 {
        interruption_missing.push(Missing::InterruptionUnknown {
            inputs: interruption_unknown,
        });
    }
    interruption_missing.extend(outcome_missing);

    // Circling is M-20's own verdict, stretch by stretch.
    let at = |uuid: &str| {
        evidence
            .records
            .iter()
            .find(|r| r.uuid == uuid)
            .map(|r| r.at_ms)
    };
    let mut circling_missing = Vec::new();
    match &evidence.repeats {
        SessionRepeats::Missing | SessionRepeats::Unmeasured { .. } => {
            circling_missing.push(Missing::CirclingUnmeasured)
        }
        SessionRepeats::Measured { stretches, .. } => {
            let mut unknown = BTreeMap::<UnknownRepeats, u64>::new();
            for stretch in stretches {
                match (&stretch.repeats, stretch.circling) {
                    (RepeatDensity::Unknown { reason }, _) => {
                        *unknown.entry(*reason).or_default() += 1
                    }
                    (RepeatDensity::Measured { repeats, worst }, Some(true)) => {
                        let mut uuids = BTreeSet::new();
                        uuids.insert(stretch.start_uuid.clone());
                        let worst = worst.as_ref().map(|group| {
                            let block = &group.representative;
                            uuids.insert(block.record_uuid.clone());
                            let tool_only = evidence
                                .tool_only_calls
                                .contains(&(block.record_uuid.clone(), block.block_index));
                            format!(
                                ", most often {} ({})",
                                call_words(&group.tool_name, tool_only),
                                times_word(group.count as usize)
                            )
                        });
                        uuids.insert(stretch.end_uuid.clone());
                        // The stretch's own start, which M-20 states whether
                        // or not its record is among these.
                        let start_ms = timestamp::parse(&stretch.start)
                            .map(|(_, ms)| ms)
                            .ok()
                            .or_else(|| at(&stretch.start_uuid))
                            .unwrap_or_default();
                        let mut record_uuids: Vec<_> = uuids.into_iter().collect();
                        record_uuids.sort_by_key(|uuid| at(uuid));
                        warnings.push(warn(
                            WarningKind::Circling,
                            start_ms,
                            record_uuids,
                            format!(
                                "went in circles: {repeats} repeated calls in {} active minutes{} (from {})",
                                stretch.active_duration_ms / 60_000,
                                worst.unwrap_or_default(),
                                clock_times(&[start_ms], zone)
                            ),
                        ));
                    }
                    (RepeatDensity::Measured { .. }, _) => {}
                }
            }
            circling_missing.extend(
                unknown
                    .into_iter()
                    .map(|(reason, stretches)| Missing::RepeatsUnknown { reason, stretches }),
            );
        }
    }

    warnings.sort_by(|a, b| a.at_ms.cmp(&b.at_ms).then(a.kind.cmp(&b.kind)));
    SessionWarnings {
        session_id: session_id.clone(),
        warnings,
        checks: WarningChecks {
            correction: CheckState::of(text_missing(
                correction_not_kept,
                correction_cut,
                correction_wrapped,
            )),
            interruption: CheckState::of(interruption_missing),
            circling: CheckState::of(circling_missing),
            repeated_prompt: CheckState::of(text_missing(
                prompt_not_kept,
                prompt_cut,
                prompt_wrapped,
            )),
            repeated_failure: CheckState::of(failure_missing),
        },
        person_messages,
        person_messages_read,
        tool_results: evidence.tool_results,
        tool_results_known: evidence.tool_results_known,
    }
}

/// Records at the same exact instant in their source's order: a record after
/// the one it names as its parent, else by UUID. `rows` is sorted by instant
/// then UUID; each run of equal instants is reordered in place. A parent
/// chain that loops keeps its UUID order.
fn source_order<T>(
    rows: Vec<T>,
    instant: impl Fn(&T) -> &InstantKey,
    uuid: impl Fn(&T) -> &str,
    parent: impl Fn(&T) -> Option<&str>,
) -> Vec<T> {
    let mut order = Vec::with_capacity(rows.len());
    let mut start = 0;
    while start < rows.len() {
        let end = start
            + rows[start..]
                .iter()
                .take_while(|row| instant(row) == instant(&rows[start]))
                .count();
        let run = &rows[start..end];
        let position: HashMap<&str, usize> = run
            .iter()
            .enumerate()
            .map(|(index, row)| (uuid(row), index))
            .collect();
        let mut children = vec![Vec::new(); run.len()];
        let mut ready = BTreeSet::new();
        for (index, row) in run.iter().enumerate() {
            match parent(row).and_then(|p| position.get(p)) {
                Some(&p) if p != index => children[p].push(index),
                _ => {
                    ready.insert(index);
                }
            }
        }
        let mut placed = vec![false; run.len()];
        while let Some(index) = ready.pop_first() {
            placed[index] = true;
            order.push(start + index);
            ready.extend(children[index].iter().copied());
        }
        order.extend((0..run.len()).filter(|i| !placed[*i]).map(|i| start + i));
        start = end;
    }
    let mut slots: Vec<Option<T>> = rows.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|index| slots[index].take())
        .collect()
}

impl MetricsDb {
    /// M-21 for every user session with an event inside the window, ordered
    /// by session id. Every rule reads only the session's records inside the
    /// window; circling is M-20 over that same window, expected repeats left
    /// out. `zone` only writes the clock times in each reason.
    ///
    /// Read inside one snapshot, so every session's rules describe the same
    /// committed state.
    pub fn session_warnings(
        &self,
        window: Window,
        thresholds: RepeatThresholds,
        zone: &TimeZone,
    ) -> Result<Vec<SessionWarnings>> {
        self.read_snapshot(|db| {
            let surfaces = hands_off::load(&db.connection, window)?;
            let mut all = Vec::new();
            for ((host, surface), sessions) in &surfaces {
                // M-09's health verdict is about the surface, so it is judged
                // once for all of its sessions, as M-20 judges it for each.
                let excluded = hands_off::health(host, surface, sessions);
                for (session_id, events) in sessions {
                    // The session's stored calls, read once for both readers.
                    let mut stored = None;
                    let repeats = db.loaded_repeats(
                        excluded.clone(),
                        Some(events.as_slice()),
                        session_id,
                        thresholds,
                        CallScope::WithoutExpectedRepeats,
                        &mut stored,
                    )?;
                    let evidence = db.warning_evidence(window, session_id, repeats, &mut stored)?;
                    all.push(decide(&evidence, zone));
                }
            }
            all.sort_by(|a, b| a.session_id.cmp(&b.session_id));
            Ok(all)
        })
    }

    fn warning_evidence(
        &self,
        window: Window,
        session_id: &str,
        repeats: SessionRepeats,
        stored: &mut Option<StoredCalls>,
    ) -> Result<Evidence> {
        struct Row {
            instant: InstantKey,
            record: EvidenceRecord,
            parent: Option<String>,
            partial: bool,
            /// May carry a tool result: stated so, or unknown.
            carrier: bool,
        }
        let start = InstantKey::from_millisecond(window.start_ms());
        let end = InstantKey::from_millisecond(window.end_ms());
        let mut rows = Vec::new();
        let mut statement = self.connection.prepare(RECORD_QUERY)?;
        let mut result = statement.query(rusqlite::params![
            session_id,
            window.start_ms(),
            window.candidate_end_ms()?,
            ToolSentKind::CodexTurnAborted.as_str(),
        ])?;
        while let Some(row) = result.next()? {
            let raw: String = row.get(1)?;
            let instant = timestamp::parse(&raw)
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?
                .0;
            if instant < start || instant >= end {
                continue;
            }
            let agent = row.get::<_, String>(3)? == "assistant";
            let turn_aborted: bool = row.get(10)?;
            rows.push(Row {
                instant,
                record: EvidenceRecord {
                    uuid: row.get(0)?,
                    at_ms: row.get(2)?,
                    agent,
                    sidechain: row.get(4)?,
                    person: if agent { Some(false) } else { row.get(5)? },
                    words: None,
                    interrupted: if agent {
                        Some(false)
                    } else if turn_aborted {
                        Some(true)
                    } else {
                        row.get(7)?
                    },
                },
                parent: row.get(9)?,
                partial: row.get(6)?,
                carrier: !agent && row.get::<_, Option<bool>>(8)? != Some(false),
            });
        }
        drop(result);
        drop(statement);
        rows.sort_by(|a, b| {
            a.instant
                .cmp(&b.instant)
                .then_with(|| a.record.uuid.cmp(&b.record.uuid))
        });
        let mut rows = source_order(
            rows,
            |row| &row.instant,
            |row| &row.record.uuid,
            |row| row.parent.as_deref(),
        );

        // A person's words, read the one way every reader reads them.
        for row in &mut rows {
            if row.record.person == Some(true) && !row.record.sidechain {
                row.record.words = Some(span_detail::person_text(
                    &self.connection,
                    &row.record.uuid,
                    row.partial,
                )?);
            }
        }

        // Saved result facts, per result record, read into outcomes here by
        // the store's one rule; and the calls they name.
        let mut saved = BTreeMap::<String, Vec<(Option<String>, Outcome)>>::new();
        let mut statement = self.connection.prepare(RESULT_QUERY)?;
        let mut result = statement.query([session_id])?;
        while let Some(row) = result.next()? {
            let wording = Wording::parse(&row.get::<_, String>(4)?);
            let version: i64 = row.get(5)?;
            let contested: bool = row.get(6)?;
            let host: Host = row.get(7)?;
            let ended = match wording {
                Some(wording) if !contested && version <= tool_result::FACT_VERSION => {
                    tool_result::outcome(row.get(2)?, row.get(3)?, wording, host)
                }
                _ => Outcome::Unknown,
            };
            saved
                .entry(row.get(0)?)
                .or_default()
                .push((row.get(1)?, ended));
        }
        drop(result);
        drop(statement);
        let mut calls = BTreeMap::<String, Vec<(String, u64)>>::new();
        let mut statement = self.connection.prepare(CALL_QUERY)?;
        let mut result = statement.query([session_id])?;
        while let Some(row) = result.next()? {
            let index: i64 = row.get(1)?;
            calls.entry(row.get(2)?).or_default().push((
                row.get(0)?,
                u64::try_from(index)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(1, index))?,
            ));
        }
        drop(result);
        drop(statement);

        let mut evidence = Evidence {
            session_id: session_id.to_owned(),
            records: Vec::new(),
            outcomes: Vec::new(),
            tool_results: 0,
            tool_results_known: 0,
            outcome_not_saved: 0,
            outcome_unknown: 0,
            repeats,
            tool_only_calls: BTreeSet::new(),
        };
        for row in rows
            .iter()
            .filter(|row| !row.record.sidechain && row.carrier)
        {
            let Some(results) = saved.get(&row.record.uuid) else {
                evidence.outcome_not_saved += 1;
                continue;
            };
            for (call_id, ended) in results {
                evidence.tool_results += 1;
                match ended {
                    Outcome::Unknown => {
                        evidence.outcome_unknown += 1;
                        continue;
                    }
                    // Neither a failure nor the person's decision.
                    Outcome::Ok | Outcome::PermissionDenied => {
                        evidence.tool_results_known += 1;
                        continue;
                    }
                    Outcome::Failed | Outcome::Declined | Outcome::Interrupted => {
                        evidence.tool_results_known += 1;
                    }
                }
                // The call this result answers: exactly one saved call with
                // its identifier, and that call's stored grouping evidence.
                let stored = self.stored_calls_once(stored, session_id)?;
                let call = match call_id.as_deref().and_then(|id| calls.get(id)) {
                    Some(found) if found.len() == 1 => {
                        let (uuid, index) = &found[0];
                        stored
                            .get(uuid)
                            .and_then(|calls| calls.iter().find(|c| c.block_index == *index))
                    }
                    _ => None,
                };
                evidence.outcomes.push(ToolOutcome {
                    result_uuid: row.record.uuid.clone(),
                    at_ms: row.record.at_ms,
                    ended: *ended,
                    call: call.map(|c| CallFacts {
                        name: c.name.clone(),
                        key: c.comparable_key().cloned(),
                        tool_only: c.names_no_argument(),
                    }),
                });
            }
        }
        // Whether each circling stretch's most repeated call names the tool
        // only, for its reason's wording.
        if let SessionRepeats::Measured { stretches, .. } = &evidence.repeats {
            for stretch in stretches.iter().filter(|s| s.circling == Some(true)) {
                let RepeatDensity::Measured {
                    worst: Some(group), ..
                } = &stretch.repeats
                else {
                    continue;
                };
                let block = &group.representative;
                let tool_only = self
                    .stored_calls_once(stored, session_id)?
                    .get(&block.record_uuid)
                    .and_then(|calls| calls.iter().find(|c| c.block_index == block.block_index))
                    .is_some_and(|call| call.names_no_argument());
                if tool_only {
                    evidence
                        .tool_only_calls
                        .insert((block.record_uuid.clone(), block.block_index));
                }
            }
        }
        evidence.records = rows.into_iter().map(|row| row.record).collect();
        Ok(evidence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        repeats::{RepeatGroup, StretchRepeats},
        session_stretches::ToolBlock,
    };

    const T0: i64 = 1_790_000_000_000; // 2026-09-21T14:13:20Z
    const MINUTE: i64 = 60_000;

    fn utc() -> TimeZone {
        TimeZone::UTC
    }

    fn agent(uuid: &str, minute: i64) -> EvidenceRecord {
        EvidenceRecord {
            uuid: uuid.into(),
            at_ms: T0 + minute * MINUTE,
            agent: true,
            sidechain: false,
            person: Some(false),
            words: None,
            interrupted: Some(false),
        }
    }
    fn person(uuid: &str, minute: i64, text: &str) -> EvidenceRecord {
        EvidenceRecord {
            uuid: uuid.into(),
            at_ms: T0 + minute * MINUTE,
            agent: false,
            sidechain: false,
            person: Some(true),
            words: Some(PersonText::Whole(text.into())),
            interrupted: Some(false),
        }
    }
    /// A person's message of which only the first part was kept.
    fn cut(uuid: &str, minute: i64, start: &str) -> EvidenceRecord {
        EvidenceRecord {
            words: Some(PersonText::Start(start.into())),
            ..person(uuid, minute, "")
        }
    }
    /// An input the classification calls automatic: a hook, a tool result, a
    /// task notification. Its words are never read.
    fn automatic(uuid: &str, minute: i64) -> EvidenceRecord {
        EvidenceRecord {
            person: Some(false),
            words: None,
            ..person(uuid, minute, "")
        }
    }
    fn evidence(records: Vec<EvidenceRecord>) -> Evidence {
        Evidence {
            session_id: "s".into(),
            records,
            outcomes: vec![],
            tool_results: 0,
            tool_results_known: 0,
            outcome_not_saved: 0,
            outcome_unknown: 0,
            repeats: SessionRepeats::Measured {
                thresholds: RepeatThresholds::default(),
                stretches: vec![],
            },
            tool_only_calls: BTreeSet::new(),
        }
    }
    fn of_kind(warnings: &SessionWarnings, kind: WarningKind) -> Vec<&SessionWarning> {
        warnings
            .warnings
            .iter()
            .filter(|w| w.kind == kind)
            .collect()
    }
    fn uuids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn still_counts_only_beside_a_negative_word() {
        assert_eq!(correction("I still need an export button"), None);
        assert_eq!(correction("still not working"), Some("still not".into()));
        assert_eq!(
            correction("It\u{2019}s still broken"),
            Some("still broken".into())
        );
        assert_eq!(
            correction("the same thing is still happening"),
            Some("still happening".into())
        );
        assert_eq!(
            correction("I still don't see it"),
            Some("still don't see".into())
        );
        // A negative word six words away is out of reach.
        assert_eq!(correction("still a b c d e broken"), None);
        assert_eq!(
            correction("still a b c d broken"),
            Some("still a b c d broken".into())
        );
    }

    #[test]
    fn correction_words_match_whole_words_only() {
        assert_eq!(correction("that's WRONG"), Some("wrong".into()));
        assert_eq!(correction("Please stop."), Some("stop".into()));
        assert_eq!(correction("try again"), Some("again".into()));
        assert_eq!(correction("I said the left panel"), Some("i said".into()));
        for none in [
            "notice the nothing",
            "a no-op change",
            "stopwatch",
            "she said yes",
            "add an export button",
        ] {
            assert_eq!(correction(none), None, "{none}");
        }
    }

    #[test]
    fn no_counts_only_as_the_first_word_of_a_short_message() {
        assert_eq!(correction("No, use the other file"), Some("no".into()));
        assert_eq!(correction("no"), Some("no".into()));
        // Eight words is still short; nine is not.
        assert_eq!(
            correction("no that is not the file I meant"),
            Some("no".into())
        );
        assert_eq!(correction("no that is not the file I meant here"), None);
        assert_eq!(correction("there is no test for it yet"), None);
    }

    #[test]
    fn short_prompts_are_not_compared_and_tag_led_ones_are_tool_written() {
        assert_eq!(
            comparable_prompt("Run the  TESTS please"),
            Some("run the tests please".into())
        );
        for short in ["yes", "go on now"] {
            assert_eq!(comparable_prompt(short), None, "{short}");
        }
        for tagged in [
            "<ci-monitor-event kind=\"check\">the build passed again</ci-monitor-event>",
            "<turn_aborted> The user interrupted the previous turn on purpose.",
            "  <task-notification><summary>done and dusted now</summary>",
        ] {
            assert!(tool_written(tagged), "{tagged}");
        }
        // A message that only mentions a tag later is a prompt like any other.
        assert!(!tool_written("fix the <div> in the header"));
        assert!(!tool_written("< 3 items"));
    }

    #[test]
    fn expected_repeats_are_named_from_the_tool_name_only() {
        for name in [
            "TodoWrite",
            "TaskUpdate",
            "update_plan",
            "mcp__ccd_turn__update_plan",
            "ScheduleWakeup",
            "Monitor",
            "sleep",
            "wait_agent",
            "SubagentHandback",
            "mcp__computer-use__app_screenshot",
            "mcp__Claude_Preview__preview_screenshot",
        ] {
            assert!(expected_repeat(name), "{name}");
        }
        // Tools whose calls name no command, path or pattern are compared
        // like any other: M-20's key decides what "the same call" is.
        for name in [
            "Bash",
            "Edit",
            "Read",
            "WebFetch",
            "WebSearch",
            "Agent",
            "Skill",
            "mcp__Claude_Browser__computer",
        ] {
            assert!(!expected_repeat(name), "{name}");
        }
    }

    #[test]
    fn a_correction_needs_an_agent_reply_before_it() {
        let found = decide(
            &evidence(vec![
                person("p1", 0, "wrong idea, start with the parser"),
                agent("a1", 1),
                person("p2", 2, "still not working"),
                person("p3", 3, "wrong file"),
            ]),
            &utc(),
        );
        let corrections = of_kind(&found, WarningKind::Correction);
        // The opening message and one sent before any new reply do not count.
        assert_eq!(corrections.len(), 1);
        assert_eq!(corrections[0].record_uuids, uuids(&["p2"]));
        assert_eq!(corrections[0].at_ms, T0 + 2 * MINUTE);
        assert_eq!(
            corrections[0].reason,
            "you wrote \"still not\" after a reply (14:15)"
        );
        assert!(found.checks.correction.is_complete());
    }

    #[test]
    fn a_tag_led_message_is_neither_a_correction_nor_the_end_of_a_reply() {
        let found = decide(
            &evidence(vec![
                person("p1", 0, "build the parser"),
                agent("a1", 1),
                // A tool wrote it: "again" and "wrong" in it are not the
                // person's, and the reply before it is still unanswered.
                person(
                    "t1",
                    2,
                    "<ci-monitor-event kind=\"check\">wrong again: still failing</ci-monitor-event>",
                ),
                person("p2", 3, "wrong file"),
                agent("a2", 4),
                person(
                    "t2",
                    5,
                    "<ci-monitor-event kind=\"check\">wrong again: still failing</ci-monitor-event>",
                ),
            ]),
            &utc(),
        );
        let found: Vec<_> = found
            .warnings
            .iter()
            .map(|w| (w.kind, w.record_uuids.clone()))
            .collect();
        // One correction (p2, still after a1's reply), and the two identical
        // tool-written messages are no repeated prompt.
        assert_eq!(found, vec![(WarningKind::Correction, uuids(&["p2"]))]);
    }

    #[test]
    fn same_prompt_twice_is_compared_without_case_or_spacing() {
        let found = decide(
            &evidence(vec![
                person("p1", 5, "Run the  tests now"),
                agent("a1", 6),
                automatic("hook", 6),
                person("p2", 12, "run the tests now\n"),
                agent("a2", 13),
                person("p3", 14, "run the tests now please"),
                agent("a3", 15),
                person("y1", 16, "yes"),
                agent("a4", 17),
                person("y2", 18, "yes"),
            ]),
            &utc(),
        );
        let repeated = of_kind(&found, WarningKind::RepeatedPrompt);
        // "yes" twice is too short to compare.
        assert_eq!(repeated.len(), 1);
        assert_eq!(repeated[0].record_uuids, uuids(&["p1", "p2"]));
        assert_eq!(repeated[0].at_ms, T0 + 12 * MINUTE);
        assert!(repeated[0].reason.starts_with("same prompt sent twice ("));
        assert!(found.checks.repeated_prompt.is_complete());
    }

    #[test]
    fn automatic_messages_never_count_as_prompts_or_corrections() {
        // Identical hook outputs and a tool result reading "wrong": the
        // stored classification says no person sent them, so neither rule
        // reads them, whatever they would have said.
        let mut hook = automatic("h1", 1);
        hook.words = Some(PersonText::Whole("wrong wrong wrong wrong".into()));
        let mut twin = automatic("h2", 2);
        twin.words = Some(PersonText::Whole("wrong wrong wrong wrong".into()));
        let found = decide(
            &evidence(vec![
                person("p1", 0, "build it"),
                agent("a1", 1),
                hook,
                twin,
                automatic("h3", 3),
                agent("a2", 4),
            ]),
            &utc(),
        );
        assert!(found.warnings.is_empty(), "{:?}", found.warnings);
        assert!(found.checks.correction.is_complete());
        assert!(found.checks.repeated_prompt.is_complete());
    }

    #[test]
    fn unkept_words_and_unknown_senders_leave_text_rules_incomplete() {
        let mut unkept = person("p2", 2, "");
        unkept.words = Some(PersonText::NotStored);
        let mut wrapped = person("p3", 4, "");
        wrapped.words = Some(PersonText::Wrapped);
        let mut unknown = person("u", 5, "");
        unknown.person = None;
        unknown.words = None;
        let found = decide(
            &evidence(vec![
                person("p1", 0, "start"),
                agent("a1", 1),
                unkept,
                agent("a2", 3),
                wrapped,
                unknown,
            ]),
            &utc(),
        );
        assert!(found.warnings.is_empty());
        let missing = CheckState::Incomplete {
            missing: vec![
                Missing::TextNotKept { messages: 1 },
                Missing::WrappedText { messages: 1 },
                Missing::Unclassified { inputs: 1 },
            ],
        };
        assert_eq!(found.checks.correction, missing);
        assert_eq!(found.checks.repeated_prompt, missing);
        assert_eq!((found.person_messages, found.person_messages_read), (3, 1));
    }

    #[test]
    fn a_cut_message_is_never_read_as_complete() {
        let long = "please look at the module layout and ".repeat(8);
        let start: String = long.chars().take(span_detail::MAX_PROMPT_CHARS).collect();
        let found = decide(
            &evidence(vec![
                person("p1", 0, "build the parser"),
                agent("a1", 1),
                // "that is wrong" came after the kept part.
                cut("c1", 2, &start),
                agent("a2", 3),
                // Its words are kept up to a correction: that one is found.
                cut("c2", 4, &format!("stop {start}")),
                agent("a3", 5),
                // Its last kept word may be cut: "stop" of "stopwatch".
                cut("c3", 6, &format!("{start} the stop")),
                agent("a4", 7),
                // Identical for their kept part: not compared.
                cut("c4", 8, &start),
            ]),
            &utc(),
        );
        let corrections = of_kind(&found, WarningKind::Correction);
        assert_eq!(corrections.len(), 1);
        assert_eq!(corrections[0].record_uuids, uuids(&["c2"]));
        assert!(of_kind(&found, WarningKind::RepeatedPrompt).is_empty());
        assert_eq!(
            found.checks.correction,
            CheckState::Incomplete {
                missing: vec![Missing::TextCut { messages: 3 }]
            }
        );
        assert_eq!(
            found.checks.repeated_prompt,
            CheckState::Incomplete {
                missing: vec![Missing::TextCut { messages: 4 }]
            }
        );
        assert_eq!((found.person_messages, found.person_messages_read), (5, 1));
        // A cut message's "no" rule cannot run: its length is not known.
        assert_eq!(find_correction("no use the other file", false), None);
    }

    fn stopped(result: &str, minute: i64, ended: Outcome) -> ToolOutcome {
        ToolOutcome {
            result_uuid: result.into(),
            at_ms: T0 + minute * MINUTE,
            ended,
            call: Some(CallFacts {
                name: "Bash".into(),
                key: Ok((1, "k".into())),
                tool_only: false,
            }),
        }
    }

    #[test]
    fn a_declined_call_and_the_marker_after_it_are_one_interruption() {
        let mut marker = automatic("i1", 3);
        marker.interrupted = Some(true);
        let mut alone = automatic("i2", 7);
        alone.interrupted = Some(true);
        let mut side = automatic("i3", 8);
        side.interrupted = Some(true);
        side.sidechain = true;
        let mut unknown = automatic("i4", 9);
        unknown.interrupted = None;
        let mut evidence = evidence(vec![
            person("p1", 0, "go"),
            agent("a1", 1),
            automatic("r1", 2),
            marker,
            agent("a2", 5),
            automatic("r2", 6),
            agent("a3", 6),
            alone,
            side,
            unknown,
            automatic("r3", 10),
        ]);
        evidence.outcomes = vec![
            stopped("r1", 2, Outcome::Declined),
            stopped("r2", 6, Outcome::Declined),
            stopped("r3", 10, Outcome::Interrupted),
        ];
        let found = decide(&evidence, &utc());
        let interruptions: Vec<_> = of_kind(&found, WarningKind::Interruption)
            .into_iter()
            .map(|w| (w.record_uuids.clone(), w.reason.as_str()))
            .collect();
        assert_eq!(
            interruptions,
            vec![
                // The marker right after the decline joins it.
                (uuids(&["r1", "i1"]), "you declined a call to Bash (14:15)"),
                // An agent reply came between: the marker is its own.
                (uuids(&["r2"]), "you declined a call to Bash (14:19)"),
                (uuids(&["i2"]), "you interrupted the agent (14:20)"),
                // A result saying the person interrupted the call.
                (uuids(&["r3"]), "you interrupted a call to Bash (14:23)"),
            ]
        );
        // Neither a declined nor an interrupted call is a failed command.
        assert!(of_kind(&found, WarningKind::RepeatedFailure).is_empty());
        assert!(found.checks.repeated_failure.is_complete());
        assert_eq!(
            found.checks.interruption,
            CheckState::Incomplete {
                missing: vec![Missing::InterruptionUnknown { inputs: 1 }]
            }
        );
    }

    #[test]
    fn a_marker_and_a_decline_at_the_same_moment_are_one_interruption() {
        // No source order between them: UUID order puts the marker first.
        let mut marker = automatic("a-marker", 2);
        marker.interrupted = Some(true);
        let mut evidence = evidence(vec![
            person("p1", 0, "go"),
            agent("a1", 1),
            marker,
            automatic("z-decline", 2),
        ]);
        evidence.outcomes = vec![stopped("z-decline", 2, Outcome::Declined)];
        let found = decide(&evidence, &utc());
        let interruptions = of_kind(&found, WarningKind::Interruption);
        assert_eq!(interruptions.len(), 1);
        assert_eq!(
            interruptions[0].record_uuids,
            uuids(&["a-marker", "z-decline"])
        );
        assert_eq!(
            interruptions[0].reason,
            "you declined a call to Bash (14:15)"
        );
        // A marker a minute before a decline is another interruption.
        let mut early = automatic("a-marker", 1);
        early.interrupted = Some(true);
        let mut apart = super::tests::evidence(vec![
            person("p1", 0, "go"),
            early,
            automatic("z-decline", 2),
        ]);
        apart.outcomes = vec![stopped("z-decline", 2, Outcome::Declined)];
        assert_eq!(
            of_kind(&decide(&apart, &utc()), WarningKind::Interruption).len(),
            2
        );
    }

    #[test]
    fn records_at_one_instant_follow_their_source_order() {
        let at = InstantKey::from_millisecond(T0);
        let later = InstantKey::from_millisecond(T0 + 1);
        // (instant, uuid, parent), sorted by instant then UUID.
        let rows = vec![
            (at.clone(), "a-marker", Some("z-decline")),
            (at.clone(), "m-other", None),
            (at.clone(), "z-decline", Some("before")),
            (later.clone(), "b", Some("c")),
            (later.clone(), "c", Some("b")),
        ];
        let ordered: Vec<_> = source_order(rows, |r| &r.0, |r| r.1, |r| r.2)
            .into_iter()
            .map(|r| r.1)
            .collect();
        // The marker after the record it follows; a loop keeps UUID order.
        assert_eq!(ordered, vec!["m-other", "z-decline", "a-marker", "b", "c"]);
    }

    #[test]
    fn codex_s_interrupted_turn_note_is_an_interruption_and_not_a_prompt() {
        // The stored proof makes the note automatic and flags it.
        let mut note = automatic("x1", 2);
        note.interrupted = Some(true);
        let found = decide(
            &evidence(vec![
                person("p1", 0, "build the thing"),
                agent("a1", 1),
                note,
                person("p2", 3, "stop, use the other branch"),
            ]),
            &utc(),
        );
        let kinds: Vec<_> = found.warnings.iter().map(|w| w.kind).collect();
        // The message after it is still after the agent's reply.
        assert_eq!(
            kinds,
            vec![WarningKind::Interruption, WarningKind::Correction]
        );
        assert_eq!(found.warnings[0].record_uuids, uuids(&["x1"]));
        assert_eq!(found.person_messages, 2);
    }

    fn failure(uuid: &str, minute: i64, key: &str) -> ToolOutcome {
        ToolOutcome {
            call: Some(CallFacts {
                name: "Bash".into(),
                key: Ok((1, key.into())),
                tool_only: false,
            }),
            ..stopped(uuid, minute, Outcome::Failed)
        }
    }

    #[test]
    fn a_tool_without_an_identifying_argument_is_named_as_the_tool() {
        let mut evidence = evidence(vec![person("p1", 0, "fix it"), agent("a1", 1)]);
        let fetch = |uuid: &str, minute| ToolOutcome {
            call: Some(CallFacts {
                name: "WebFetch".into(),
                key: Ok((1, "webfetch-any".into())),
                tool_only: true,
            }),
            ..failure(uuid, minute, "")
        };
        evidence.outcomes = vec![fetch("r1", 2), fetch("r2", 3)];
        let found = decide(&evidence, &utc());
        assert_eq!(
            of_kind(&found, WarningKind::RepeatedFailure)[0].reason,
            "WebFetch failed twice (14:15, 14:16)"
        );
    }

    #[test]
    fn the_same_call_failing_twice_is_one_warning() {
        let mut evidence = evidence(vec![person("p1", 0, "fix it"), agent("a1", 1)]);
        evidence.outcomes = vec![
            failure("r1", 2, "pnpm-test"),
            failure("r2", 3, "cargo-build"),
            failure("r3", 4, "pnpm-test"),
            failure("r4", 5, "pnpm-test"),
            ToolOutcome {
                call: Some(CallFacts {
                    name: "Bash".into(),
                    key: Err(UnknownRepeats::MissingKey),
                    tool_only: false,
                }),
                ..failure("r5", 6, "")
            },
        ];
        let found = decide(&evidence, &utc());
        let failed = of_kind(&found, WarningKind::RepeatedFailure);
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].record_uuids, uuids(&["r1", "r3", "r4"]));
        assert!(
            failed[0]
                .reason
                .starts_with("the same Bash call failed 3 times"),
            "{}",
            failed[0].reason
        );
        assert_eq!(
            found.checks.repeated_failure,
            CheckState::Incomplete {
                missing: vec![Missing::FailedCallKey {
                    reason: UnknownRepeats::MissingKey,
                    calls: 1
                }]
            }
        );
    }

    #[test]
    fn a_result_naming_an_unknown_call_is_counted_once_and_only_for_failures() {
        let mut evidence = evidence(vec![
            person("p1", 0, "fix it"),
            agent("a1", 1),
            automatic("r1", 2),
            automatic("r2", 3),
        ]);
        evidence.outcomes = vec![
            ToolOutcome {
                call: None,
                ..failure("r1", 2, "")
            },
            ToolOutcome {
                call: None,
                ..stopped("r2", 3, Outcome::Declined)
            },
        ];
        let found = decide(&evidence, &utc());
        assert_eq!(
            found.checks.repeated_failure,
            CheckState::Incomplete {
                missing: vec![Missing::UnmatchedResults { results: 1 }]
            }
        );
        // The decline was found; only its tool's name is not known.
        assert!(found.checks.interruption.is_complete());
        assert_eq!(
            of_kind(&found, WarningKind::Interruption)[0].reason,
            "you declined a call to a tool (14:16)"
        );
    }

    #[test]
    fn unsaved_or_unknown_outcomes_leave_failure_and_decline_checks_incomplete() {
        let mut evidence = evidence(vec![person("p1", 0, "fix it"), agent("a1", 1)]);
        evidence.outcome_not_saved = 2;
        evidence.outcome_unknown = 3;
        let found = decide(&evidence, &utc());
        let missing = CheckState::Incomplete {
            missing: vec![
                Missing::OutcomeNotSaved { records: 2 },
                Missing::OutcomeUnknown { results: 3 },
            ],
        };
        assert_eq!(found.checks.repeated_failure, missing);
        assert_eq!(found.checks.interruption, missing);
        assert!(found.checks.correction.is_complete());
    }

    fn stretch(
        start: &str,
        end: &str,
        repeats: RepeatDensity,
        circling: Option<bool>,
    ) -> StretchRepeats {
        StretchRepeats {
            start_uuid: start.into(),
            end_uuid: end.into(),
            start: Timestamp::from_millisecond(T0).unwrap().to_string(),
            end: Timestamp::from_millisecond(T0 + 10 * MINUTE)
                .unwrap()
                .to_string(),
            duration_ms: 600_000,
            active_duration_ms: 540_000,
            repeats,
            circling,
        }
    }
    fn bash_group(count: u64) -> RepeatDensity {
        RepeatDensity::Measured {
            repeats: 6,
            worst: Some(RepeatGroup {
                tool_name: "Bash".into(),
                count,
                representative: ToolBlock {
                    record_uuid: "a1".into(),
                    block_index: 0,
                },
            }),
        }
    }

    #[test]
    fn circling_is_m20_s_own_verdict() {
        let mut evidence = evidence(vec![
            person("p1", 0, "go"),
            agent("a1", 1),
            agent("a2", 9),
            person("p2", 20, "next"),
            agent("a3", 21),
        ]);
        evidence.repeats = SessionRepeats::Measured {
            thresholds: RepeatThresholds::default(),
            stretches: vec![
                stretch("p1", "a2", bash_group(5), Some(true)),
                // Measured and not circling: no warning, nothing missing.
                stretch(
                    "p2",
                    "a3",
                    RepeatDensity::Measured {
                        repeats: 1,
                        worst: None,
                    },
                    Some(false),
                ),
            ],
        };
        let found = decide(&evidence, &utc());
        let circling = of_kind(&found, WarningKind::Circling);
        assert_eq!(circling.len(), 1);
        assert_eq!(circling[0].record_uuids, uuids(&["p1", "a1", "a2"]));
        assert_eq!(circling[0].at_ms, T0);
        assert!(
            circling[0].reason.starts_with(
                "went in circles: 6 repeated calls in 9 active minutes, most often the same Bash call (5 times)"
            ),
            "{}",
            circling[0].reason
        );
        assert!(found.checks.circling.is_complete());

        // An uncounted stretch and an unmeasured session are unknowns.
        evidence.repeats = SessionRepeats::Measured {
            thresholds: RepeatThresholds::default(),
            stretches: vec![stretch(
                "p1",
                "a2",
                RepeatDensity::Unknown {
                    reason: UnknownRepeats::MissingKey,
                },
                None,
            )],
        };
        assert_eq!(
            decide(&evidence, &utc()).checks.circling,
            CheckState::Incomplete {
                missing: vec![Missing::RepeatsUnknown {
                    reason: UnknownRepeats::MissingKey,
                    stretches: 1
                }]
            }
        );
        evidence.repeats = SessionRepeats::Unmeasured {
            excluded_surface: None,
        };
        assert_eq!(
            decide(&evidence, &utc()).checks.circling,
            CheckState::Incomplete {
                missing: vec![Missing::CirclingUnmeasured]
            }
        );
    }

    #[test]
    fn a_circling_warning_starts_when_its_stretch_does() {
        // The stretch's start record is not among the evidence records (it
        // began before the records this read holds): M-20's own start time
        // is used, never zero.
        let mut evidence = evidence(vec![agent("a1", 1), agent("a2", 9)]);
        evidence.repeats = SessionRepeats::Measured {
            thresholds: RepeatThresholds::default(),
            stretches: vec![stretch("gone", "a2", bash_group(5), Some(true))],
        };
        evidence.tool_only_calls.insert(("a1".into(), 0));
        let found = decide(&evidence, &utc());
        let circling = of_kind(&found, WarningKind::Circling);
        assert_eq!(circling[0].at_ms, T0);
        assert!(
            circling[0].reason.contains("(from 14:13)"),
            "{}",
            circling[0].reason
        );
        // Its most repeated call names only the tool.
        assert!(
            circling[0].reason.contains(", most often Bash (5 times)"),
            "{}",
            circling[0].reason
        );
    }

    #[test]
    fn times_on_another_day_carry_their_date() {
        let text = clock_times(&[T0, T0 + 5 * MINUTE, T0 + 24 * 60 * MINUTE], &utc());
        assert_eq!(text, "14:13, 14:18, Sep 22 14:13");
    }

    #[test]
    fn the_evidence_reads_touch_no_content_and_no_argument() {
        for query in [RECORD_QUERY, RESULT_QUERY, CALL_QUERY] {
            for forbidden in ["content_json", "input_json", "command", "file_path"] {
                assert!(!query.contains(forbidden), "{forbidden}");
            }
        }
    }
}
