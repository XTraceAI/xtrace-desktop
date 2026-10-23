//! Count M-21 session warnings in one database, read-only.
//!
//! `cargo run -p xt-metrics --example session_warnings -- <database>`
//!
//! The window is the database's whole recorded history. Prints how many
//! sessions were read, how many have at least one warning, the count per
//! kind, how many sessions each rule could not fully check and why, and up to
//! eight example sessions per kind. No message text is printed except the
//! few words a correction matched.

use jiff::tz::TimeZone;
use rusqlite::{Connection, OpenFlags};
use std::collections::{BTreeMap, BTreeSet};
use xt_metrics::{CheckState, MetricsDb, Missing, RepeatThresholds, WarningKind, Window};

const EXAMPLES: usize = 8;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: session_warnings <database>")?;
    let metrics = MetricsDb::open(&path)?;
    let side = Connection::open_with_flags(
        &path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    side.pragma_update(None, "query_only", true)?;
    let (first, last): (Option<i64>, Option<i64>) = side.query_row(
        "SELECT min(ts_ms),max(ts_ms) FROM v_session_events",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let (Some(first), Some(last)) = (first, last) else {
        println!("no timed events");
        return Ok(());
    };
    // The exact instant can sit up to a second past its millisecond
    // projection around a leap second; end well after the last one.
    let window = Window::new(first, last + 2_000)?;
    let zone = TimeZone::system();
    let started = std::time::Instant::now();
    let all = metrics.session_warnings(window, RepeatThresholds::default(), &zone)?;
    let elapsed = started.elapsed();

    let host = |id: &str| -> rusqlite::Result<(String, Option<String>)> {
        side.query_row(
            "SELECT host,nullif(trim(title),'') FROM sessions WHERE session_id=?1",
            [id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
    };
    let mut hosts = BTreeMap::<String, String>::new();
    for session in &all {
        hosts.insert(session.session_id.clone(), host(&session.session_id)?.0);
    }

    println!("Session warnings (M-21), read-only");
    println!(
        "window: {} .. {} (whole history), read in {:.1} s",
        jiff::Timestamp::from_millisecond(first)?,
        jiff::Timestamp::from_millisecond(last)?,
        elapsed.as_secs_f64()
    );
    let mut per_host = BTreeMap::<&str, (u64, u64)>::new();
    for session in &all {
        let entry = per_host.entry(&hosts[&session.session_id]).or_default();
        entry.0 += 1;
        entry.1 += u64::from(!session.warnings.is_empty());
    }
    println!();
    println!("sessions read: {}", all.len());
    println!(
        "sessions with at least one warning: {}",
        all.iter().filter(|s| !s.warnings.is_empty()).count()
    );
    for (host, (read, warned)) in &per_host {
        println!("  {host}: {read} read, {warned} with a warning");
    }

    println!();
    println!("per kind: warnings / sessions with one / sessions not fully checked");
    for kind in WarningKind::ALL {
        let warnings: usize = all
            .iter()
            .map(|s| s.warnings.iter().filter(|w| w.kind == kind).count())
            .sum();
        let sessions = all
            .iter()
            .filter(|s| s.warnings.iter().any(|w| w.kind == kind))
            .count();
        let incomplete = all
            .iter()
            .filter(|s| !s.checks.get(kind).is_complete())
            .count();
        println!("  {kind:?}: {warnings} / {sessions} / {incomplete}");
        let mut by_host = BTreeMap::<&str, usize>::new();
        for s in all
            .iter()
            .filter(|s| s.warnings.iter().any(|w| w.kind == kind))
        {
            *by_host.entry(&hosts[&s.session_id]).or_default() += 1;
        }
        if !by_host.is_empty() {
            println!("      sessions by app: {by_host:?}");
        }
    }

    println!();
    println!("why rules could not fully check a session (sessions per reason):");
    for kind in WarningKind::ALL {
        let mut reasons = BTreeMap::<String, usize>::new();
        for session in &all {
            if let CheckState::Incomplete { missing } = session.checks.get(kind) {
                let names: BTreeSet<_> = missing.iter().map(reason_name).collect();
                for name in names {
                    *reasons.entry(name).or_default() += 1;
                }
            }
        }
        println!("  {kind:?}: {reasons:?}");
    }
    let text_none = all
        .iter()
        .filter(|s| s.person_messages > 0 && s.person_messages_read == 0)
        .count();
    let no_person = all.iter().filter(|s| s.person_messages == 0).count();
    let (sent, read) = all.iter().fold((0, 0), |(sent, read), s| {
        (sent + s.person_messages, read + s.person_messages_read)
    });
    println!();
    println!(
        "sessions where the text rules (correction, repeated prompt) could not fully run: {}",
        all.iter()
            .filter(
                |s| !s.checks.correction.is_complete() || !s.checks.repeated_prompt.is_complete()
            )
            .count()
    );
    println!("  of which no person's message words were readable at all: {text_none}");
    println!("sessions with no person's message in the window: {no_person}");
    println!("person's messages: {sent}, words readable for {read}");
    let (results, known) = all.iter().fold((0, 0), |(results, known), s| {
        (results + s.tool_results, known + s.tool_results_known)
    });
    println!(
        "tool results with a saved outcome: {results}, known outcome for {known}; \
         sessions where every result had a known outcome: {}",
        all.iter()
            .filter(|s| s.tool_results > 0 && s.tool_results == s.tool_results_known)
            .count()
    );

    // Which correction words matched: the quoted words of each reason.
    let mut matched = BTreeMap::<String, usize>::new();
    for warning in all
        .iter()
        .flat_map(|s| &s.warnings)
        .filter(|w| w.kind == WarningKind::Correction)
    {
        if let Some(phrase) = warning.reason.split('"').nth(1) {
            let word = if phrase.starts_with("still") || phrase.ends_with("still") {
                "still + negative"
            } else {
                phrase
            };
            *matched.entry(word.to_owned()).or_default() += 1;
        }
    }
    println!();
    println!("correction warnings by matched word: {matched:?}");

    for kind in WarningKind::ALL {
        println!();
        println!("examples: {kind:?}");
        let mut shown = 0;
        for session in &all {
            let Some(first) = session.warnings.iter().find(|w| w.kind == kind) else {
                continue;
            };
            let more = session.warnings.iter().filter(|w| w.kind == kind).count() - 1;
            let (host, title) = host(&session.session_id)?;
            println!(
                "  {} [{host}] {:?} at {}: {}{}",
                session.session_id,
                title.as_deref().unwrap_or("(no saved title)"),
                jiff::Timestamp::from_millisecond(first.at_ms)?
                    .to_zoned(zone.clone())
                    .strftime("%Y-%m-%d %H:%M"),
                first.reason,
                if more > 0 {
                    format!(" (+{more} more)")
                } else {
                    String::new()
                }
            );
            shown += 1;
            if shown == EXAMPLES {
                break;
            }
        }
        if shown == 0 {
            println!("  none");
        }
    }
    Ok(())
}

fn reason_name(missing: &Missing) -> String {
    match missing {
        Missing::TextNotKept { .. } => "words not kept".into(),
        Missing::TextCut { .. } => "only the first 280 characters kept".into(),
        Missing::WrappedText { .. } => "only part of message is the person's".into(),
        Missing::Unclassified { .. } => "sender unknown".into(),
        Missing::InterruptionUnknown { .. } => "interruption flag unknown".into(),
        Missing::OutcomeNotSaved { .. } => "tool outcome not saved".into(),
        Missing::OutcomeUnknown { .. } => "tool outcome unknown".into(),
        Missing::UnmatchedResults { .. } => "result names an unknown call".into(),
        Missing::FailedCallKey { reason, .. } => format!("failed call key: {reason:?}"),
        Missing::CirclingUnmeasured => "M-20 has no stretches for it".into(),
        Missing::RepeatsUnknown { reason, .. } => format!("M-20 repeats unknown: {reason:?}"),
    }
}
