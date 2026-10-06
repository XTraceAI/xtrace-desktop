//! "Your hours": the stretches of a day a person was working with agents,
//! read from the times they sent messages.
//!
//! Every message the app counts as a person's (`human_is_eligible`, exactly
//! the messages [`crate::Counts::human_messages`] counts) is placed on one
//! timeline, across every conversation and tool, in time order. Two
//! consecutive messages at most the break length apart join one stretch, and
//! the whole gap between them counts; a longer gap is a break. A stretch is
//! split at local midnight. A message with no neighbour within the break
//! length is a stretch of zero length: it adds nothing, and is still returned
//! so a display can mark it.
//!
//! Your hours cover whole local days (M-08): from local midnight of the
//! selected range's first local day through the end of its last, and the
//! previous period is the same number of whole local days just before. Each
//! read also takes the messages of up to one break length before a period, so
//! a stretch that began before its first midnight keeps its part after
//! midnight instead of being lost; the two periods share one timeline, so a
//! stretch over their boundary is split between them, never dropped by both.
//!
//! No message text is read and nothing is filtered by content: which messages
//! are a person's is the ingest owner's classification, as for every Human
//! count. A message whose classification is unknown, inside a period or
//! within a break length before or after it, makes that period unknown, as it
//! makes the Human message count unknown, never a smaller number.
use crate::{Error, MetricsDb, Result, Window};
use jiff::tz::TimeZone;
use serde::Serialize;
use xt_store::timestamp::{self, InstantKey};

pub(crate) const QUERY: &str =
    "SELECT ts,ts_ms,human_is_eligible FROM v_session_events WHERE ts_ms>=?1 AND ts_ms<?2";

const MINUTE_MS: i64 = 60_000;

/// The longest gap between two consecutive messages that still counts as one
/// stretch, in whole positive minutes. The saved setting owns its bounds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BreakLength(u32);
impl BreakLength {
    pub fn new(minutes: u32) -> Result<Self> {
        if minutes == 0 {
            return Err(Error::InvalidBreakLength);
        }
        Ok(Self(minutes))
    }
    pub fn minutes(self) -> u32 {
        self.0
    }
    fn ms(self) -> i64 {
        i64::from(self.0) * MINUTE_MS
    }
}
impl Default for BreakLength {
    fn default() -> Self {
        Self(60)
    }
}

/// One active stretch inside one local day, in UTC milliseconds. `end_ms`
/// equals `start_ms` for a single message; a stretch cut at midnight ends at
/// the next day's start, and its next piece starts there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct HumanStretch {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// One whole local day: its stretches in time order and their total. Both
/// are unknown (`None`, no stretches) on every day when any message the
/// period reads has an unknown classification.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct DayHumanHours {
    pub date: String,
    pub start_ms: i64,
    pub end_ms: i64,
    pub active_ms: Option<u64>,
    pub stretches: Vec<HumanStretch>,
}

/// Your hours over one period of whole local days.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HumanHours {
    pub break_minutes: u32,
    /// The period's first local midnight.
    pub start_ms: i64,
    /// The local midnight after its last day.
    pub end_ms: i64,
    /// The days' totals added up; `None` when a classification is unknown.
    pub active_ms: Option<u64>,
    /// Messages sent inside the period; `None` when a classification is unknown.
    pub messages: Option<u64>,
    pub by_day: Vec<DayHumanHours>,
}

/// Your hours for the whole local days a selected range touches, and for the
/// same number of whole local days just before, from one timeline.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HumanHoursPeriods {
    pub current: HumanHours,
    pub previous: HumanHours,
}

/// One period's days as `(date, start_ms, end_ms)`, oldest first.
type Days = Vec<(String, i64, i64)>;

/// The shared stretches and the sorted message times they were joined from.
pub(crate) type Known<'a> = (&'a [(i64, i64)], &'a [i64]);

fn days_of(period: Window, zone: TimeZone) -> Result<Days> {
    Ok(period
        .local_days(zone)?
        .into_iter()
        .map(|day| {
            (
                day.date.to_string(),
                day.window.start_ms(),
                day.window.end_ms(),
            )
        })
        .collect())
}

impl MetricsDb {
    /// Your hours for `selected` widened to whole local days, and for the
    /// previous period of the same number of whole local days. Exact source
    /// instants decide which messages are read, as for the Human message
    /// count; the timeline itself is on the POSIX millisecond axis.
    pub fn human_hours(
        &self,
        selected: Window,
        break_length: BreakLength,
        zone: TimeZone,
    ) -> Result<HumanHoursPeriods> {
        let current = selected.whole_local_days(zone.clone())?;
        let previous = current.previous_local_days(zone.clone())?;
        let gap = break_length.ms();
        let read_from = previous
            .start_ms()
            .checked_sub(gap)
            .ok_or(Error::InvalidWindow)?;
        let read = Window::new(read_from, current.end_ms())?;
        let start = InstantKey::from_millisecond(read.start_ms());
        let end = InstantKey::from_millisecond(read.end_ms());
        let mut times = Vec::new();
        let mut unknown = Vec::new();
        self.read_snapshot(|db| {
            let mut statement = db.connection.prepare(QUERY)?;
            let mut rows = statement.query([read.start_ms(), read.candidate_end_ms()?])?;
            while let Some(row) = rows.next()? {
                let raw: String = row.get(0)?;
                let instant = timestamp::parse(&raw)
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?
                    .0;
                if instant < start || instant >= end {
                    continue;
                }
                // A leap second's projection can land on the read's end; it
                // stays inside the last day.
                let ms = row
                    .get::<_, i64>(1)?
                    .clamp(read.start_ms(), read.end_ms() - 1);
                match row.get::<_, Option<bool>>(2)? {
                    Some(false) => {}
                    None => unknown.push(ms),
                    Some(true) => times.push(ms),
                }
            }
            Ok(())
        })?;
        let stretches = join(&mut times, break_length);
        let period = |window: Window| -> Result<HumanHours> {
            // An unknown message up to a break length before the period could
            // start a stretch that reaches into it, and one up to a break
            // length after could extend a stretch that started inside it.
            let reads_unknown = unknown
                .iter()
                .any(|ms| *ms >= window.start_ms() - gap && *ms < window.end_ms() + gap);
            Ok(split(
                (!reads_unknown).then_some((stretches.as_slice(), times.as_slice())),
                break_length,
                window,
                &days_of(window, zone.clone())?,
            ))
        };
        Ok(HumanHoursPeriods {
            current: period(current)?,
            previous: period(previous)?,
        })
    }
}

/// Sort message times and join them into stretches: consecutive times at
/// most the break length apart share one.
pub(crate) fn join(times: &mut [i64], break_length: BreakLength) -> Vec<(i64, i64)> {
    times.sort_unstable();
    let gap = break_length.ms();
    let mut stretches: Vec<(i64, i64)> = Vec::new();
    for &time in times.iter() {
        match stretches.last_mut() {
            Some((_, end)) if time - *end <= gap => *end = time,
            _ => stretches.push((time, time)),
        }
    }
    stretches
}

/// One period's days from the shared stretches: each stretch clipped to each
/// day, so a part before the period's first midnight is left out and the rest
/// kept. `None` is an unknown classification. Pure, so the rule is tested
/// apart from storage.
pub(crate) fn split(
    known: Option<Known<'_>>,
    break_length: BreakLength,
    period: Window,
    days: &[(String, i64, i64)],
) -> HumanHours {
    let mut by_day: Vec<DayHumanHours> = days
        .iter()
        .map(|(date, start_ms, end_ms)| DayHumanHours {
            date: date.clone(),
            start_ms: *start_ms,
            end_ms: *end_ms,
            active_ms: known.map(|_| 0),
            stretches: Vec::new(),
        })
        .collect();
    let Some((stretches, times)) = known else {
        return HumanHours {
            break_minutes: break_length.minutes(),
            start_ms: period.start_ms(),
            end_ms: period.end_ms(),
            active_ms: None,
            messages: None,
            by_day,
        };
    };
    for &(start, end) in stretches {
        if end < period.start_ms() || start >= period.end_ms() {
            continue;
        }
        for day in &mut by_day {
            let piece = if start == end {
                // A single message belongs to the day that contains it.
                (day.start_ms <= start && start < day.end_ms).then_some((start, end))
            } else {
                let from = start.max(day.start_ms);
                let to = end.min(day.end_ms);
                (from < to).then_some((from, to))
            };
            if let Some((from, to)) = piece {
                day.stretches.push(HumanStretch {
                    start_ms: from,
                    end_ms: to,
                });
                day.active_ms = day.active_ms.map(|ms| ms + (to - from) as u64);
            }
        }
    }
    let messages = times
        .iter()
        .filter(|ms| **ms >= period.start_ms() && **ms < period.end_ms())
        .count() as u64;
    HumanHours {
        break_minutes: break_length.minutes(),
        start_ms: period.start_ms(),
        end_ms: period.end_ms(),
        active_ms: Some(by_day.iter().filter_map(|day| day.active_ms).sum()),
        messages: Some(messages),
        by_day,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const H: i64 = 3_600_000;
    const M: i64 = MINUTE_MS;
    /// Two UTC days starting at 0 and at 24 h.
    fn days() -> Vec<(String, i64, i64)> {
        vec![
            ("2026-09-01".into(), 0, 24 * H),
            ("2026-09-02".into(), 24 * H, 48 * H),
        ]
    }
    /// The stretches of `times`, split into `days` as one period.
    fn timeline(
        times: Option<Vec<i64>>,
        break_length: BreakLength,
        days: &[(String, i64, i64)],
    ) -> HumanHours {
        let period = Window::new(days[0].1, days[days.len() - 1].2).unwrap();
        match times {
            None => split(None, break_length, period, days),
            Some(mut times) => {
                let stretches = join(&mut times, break_length);
                split(Some((&stretches, &times)), break_length, period, days)
            }
        }
    }
    fn run(times: &[i64], minutes: u32) -> HumanHours {
        timeline(
            Some(times.to_vec()),
            BreakLength::new(minutes).unwrap(),
            &days(),
        )
    }
    fn spans(day: &DayHumanHours) -> Vec<(i64, i64)> {
        day.stretches
            .iter()
            .map(|s| (s.start_ms, s.end_ms))
            .collect()
    }

    #[test]
    fn a_gap_of_exactly_the_break_length_counts_as_active() {
        let report = run(&[10 * H, 11 * H], 60);
        assert_eq!(spans(&report.by_day[0]), [(10 * H, 11 * H)]);
        assert_eq!(report.by_day[0].active_ms, Some(H as u64));
        assert_eq!(report.active_ms, Some(H as u64));
    }

    #[test]
    fn one_minute_more_is_a_break() {
        let report = run(&[10 * H, 11 * H + M], 60);
        assert_eq!(
            spans(&report.by_day[0]),
            [(10 * H, 10 * H), (11 * H + M, 11 * H + M)]
        );
        assert_eq!(report.by_day[0].active_ms, Some(0));
        assert_eq!(report.active_ms, Some(0));
        assert_eq!(report.messages, Some(2));
    }

    #[test]
    fn a_stretch_over_midnight_is_split_between_the_two_days() {
        let report = run(&[23 * H + 30 * M, 24 * H + 20 * M], 60);
        assert_eq!(spans(&report.by_day[0]), [(23 * H + 30 * M, 24 * H)]);
        assert_eq!(spans(&report.by_day[1]), [(24 * H, 24 * H + 20 * M)]);
        assert_eq!(report.by_day[0].active_ms, Some((30 * M) as u64));
        assert_eq!(report.by_day[1].active_ms, Some((20 * M) as u64));
        assert_eq!(report.active_ms, Some((50 * M) as u64));
    }

    #[test]
    fn a_stretch_ending_at_midnight_leaves_no_piece_on_the_next_day() {
        let report = run(&[23 * H, 24 * H], 60);
        assert_eq!(spans(&report.by_day[0]), [(23 * H, 24 * H)]);
        assert!(report.by_day[1].stretches.is_empty());
        assert_eq!(report.by_day[1].active_ms, Some(0));
    }

    #[test]
    fn interleaved_conversations_share_one_timeline() {
        // Two conversations alternating, each 50 minutes between its own
        // messages but 25 between any two on the merged timeline, at a
        // 30-minute break length: one stretch, not two sets of single ticks.
        let a = [9 * H, 9 * H + 50 * M, 10 * H + 40 * M];
        let b = [9 * H + 25 * M, 10 * H + 15 * M];
        let mut all = [b.as_slice(), a.as_slice()].concat();
        all.reverse();
        let report = run(&all, 30);
        assert_eq!(spans(&report.by_day[0]), [(9 * H, 10 * H + 40 * M)]);
        assert_eq!(report.active_ms, Some((100 * M) as u64));
        assert_eq!(report.messages, Some(5));
    }

    #[test]
    fn an_isolated_message_is_a_zero_length_stretch() {
        let report = run(&[8 * H, 14 * H, 14 * H + 10 * M], 60);
        assert_eq!(
            spans(&report.by_day[0]),
            [(8 * H, 8 * H), (14 * H, 14 * H + 10 * M)]
        );
        assert_eq!(report.by_day[0].active_ms, Some((10 * M) as u64));
        // At midnight exactly, a single message is the new day's.
        let midnight = run(&[24 * H], 60);
        assert!(midnight.by_day[0].stretches.is_empty());
        assert_eq!(spans(&midnight.by_day[1]), [(24 * H, 24 * H)]);
    }

    #[test]
    fn duplicate_times_join_their_stretch() {
        let report = run(&[5 * H, 5 * H, 5 * H + 5 * M], 60);
        assert_eq!(spans(&report.by_day[0]), [(5 * H, 5 * H + 5 * M)]);
    }

    #[test]
    fn an_empty_range_is_a_measured_zero_on_every_day() {
        let report = run(&[], 60);
        assert_eq!(report.active_ms, Some(0));
        assert_eq!(report.messages, Some(0));
        assert_eq!(report.by_day.len(), 2);
        for day in &report.by_day {
            assert_eq!(day.active_ms, Some(0));
            assert!(day.stretches.is_empty());
        }
    }

    #[test]
    fn an_unknown_classification_makes_every_day_unknown() {
        let report = timeline(None, BreakLength::default(), &days());
        assert_eq!((report.start_ms, report.end_ms), (0, 48 * H));
        assert_eq!(report.active_ms, None);
        assert_eq!(report.messages, None);
        assert_eq!(report.break_minutes, 60);
        for day in &report.by_day {
            assert_eq!(day.active_ms, None);
            assert!(day.stretches.is_empty());
        }
    }

    #[test]
    fn a_stretch_from_before_the_period_keeps_its_part_after_midnight() {
        // 23:50 the evening before the period and 00:20 on its first day:
        // the 20 minutes after midnight count on the first day.
        let report = run(&[-10 * M, 20 * M], 60);
        assert_eq!(spans(&report.by_day[0]), [(0, 20 * M)]);
        assert_eq!(report.by_day[0].active_ms, Some((20 * M) as u64));
        assert_eq!(report.active_ms, Some((20 * M) as u64));
        // Only the message inside the period is the period's.
        assert_eq!(report.messages, Some(1));
        // A message before the period that joins nothing adds nothing.
        let alone = run(&[-2 * H, 5 * H], 60);
        assert_eq!(spans(&alone.by_day[0]), [(5 * H, 5 * H)]);
        assert_eq!(alone.messages, Some(1));
    }

    #[test]
    fn two_periods_from_one_timeline_split_a_stretch_over_their_boundary() {
        // The previous period is the first day, the current one the second;
        // 23:30 and 00:15 join one stretch, split between them, none lost.
        let mut times = vec![23 * H + 30 * M, 24 * H + 15 * M];
        let gap = BreakLength::default();
        let stretches = join(&mut times, gap);
        let all = days();
        let previous = split(
            Some((&stretches, &times)),
            gap,
            Window::new(0, 24 * H).unwrap(),
            &all[..1],
        );
        let current = split(
            Some((&stretches, &times)),
            gap,
            Window::new(24 * H, 48 * H).unwrap(),
            &all[1..],
        );
        assert_eq!(previous.active_ms, Some((30 * M) as u64));
        assert_eq!(current.active_ms, Some((15 * M) as u64));
        assert_eq!(spans(&current.by_day[0]), [(24 * H, 24 * H + 15 * M)]);
        assert_eq!((previous.messages, current.messages), (Some(1), Some(1)));
    }

    #[test]
    fn the_break_length_is_positive_whole_minutes() {
        assert!(matches!(
            BreakLength::new(0),
            Err(Error::InvalidBreakLength)
        ));
        assert_eq!(BreakLength::new(5).unwrap().minutes(), 5);
        assert_eq!(BreakLength::default().minutes(), 60);
    }
}
