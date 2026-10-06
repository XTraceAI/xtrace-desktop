use crate::{Error, Result};
use jiff::{Timestamp, civil::Date, tz::TimeZone};

/// Half-open [start_ms, end_ms). Equal-length previous windows are elapsed-time
/// comparisons; calendar-day buckets may be shorter or longer around DST.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Window {
    start_ms: i64,
    end_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DayBucket {
    pub date: Date,
    pub window: Window,
}

impl Window {
    pub fn new(start_ms: i64, end_ms: i64) -> Result<Self> {
        if start_ms >= end_ms || end_ms.checked_sub(start_ms).is_none() {
            return Err(Error::InvalidWindow);
        }
        Timestamp::from_millisecond(start_ms)?;
        Timestamp::from_millisecond(end_ms)?;
        Ok(Self { start_ms, end_ms })
    }
    pub fn start_ms(self) -> i64 {
        self.start_ms
    }
    pub fn end_ms(self) -> i64 {
        self.end_ms
    }
    // Chrono's coarse projection puts a leap second into the following POSIX
    // second. Include that overlap, then filter exact instants before accumulation.
    pub(crate) fn candidate_end_ms(self) -> Result<i64> {
        self.end_ms().checked_add(1000).ok_or(Error::InvalidWindow)
    }

    pub fn previous(self) -> Result<Self> {
        let start = self
            .start_ms
            .checked_sub(self.end_ms - self.start_ms)
            .ok_or(Error::InvalidWindow)?;
        Self::new(start, self.start_ms)
    }

    /// The complete Monday-to-Monday ISO week containing the explicit anchor
    /// in the supplied zone. Calendar arithmetic preserves DST transitions.
    pub fn iso_week(anchor_ms: i64, zone: TimeZone) -> Result<Self> {
        let local = Timestamp::from_millisecond(anchor_ms)?.to_zoned(zone);
        let monday = local
            .start_of_day()?
            .checked_sub(
                jiff::Span::new().days(i64::from(local.weekday().to_monday_zero_offset())),
            )?
            .start_of_day()?;
        let next = monday
            .checked_add(jiff::Span::new().days(7))?
            .start_of_day()?;
        Self::new(
            monday.timestamp().as_millisecond(),
            next.timestamp().as_millisecond(),
        )
    }

    /// The whole local days this window touches: from local midnight of its
    /// first day to local midnight after its last day. The days are the same
    /// dates [`Window::local_days`] reports, none of them clipped.
    pub fn whole_local_days(self, zone: TimeZone) -> Result<Self> {
        let first = Timestamp::from_millisecond(self.start_ms)?
            .to_zoned(zone.clone())
            .start_of_day()?;
        let after_last = Timestamp::from_millisecond(self.end_ms - 1)?
            .to_zoned(zone)
            .start_of_day()?
            .tomorrow()?
            .start_of_day()?;
        Self::new(
            first.timestamp().as_millisecond(),
            after_last.timestamp().as_millisecond(),
        )
    }

    /// The same number of local days as this window touches, just before its
    /// first local midnight; for a whole-day window, the previous period of
    /// the same number of whole days. Calendar arithmetic keeps DST days whole.
    pub fn previous_local_days(self, zone: TimeZone) -> Result<Self> {
        let days = i64::try_from(self.local_days(zone.clone())?.len())
            .map_err(|_| Error::InvalidWindow)?;
        let first = Timestamp::from_millisecond(self.start_ms)?
            .to_zoned(zone)
            .start_of_day()?;
        let start = first
            .checked_sub(jiff::Span::new().days(days))?
            .start_of_day()?;
        Self::new(
            start.timestamp().as_millisecond(),
            first.timestamp().as_millisecond(),
        )
    }

    /// Split the selected interval at local calendar boundaries, clipping the
    /// first and final days. Callers supply the zone and now; no implicit clock.
    pub fn local_days(self, zone: TimeZone) -> Result<Vec<DayBucket>> {
        let mut buckets = Vec::new();
        let mut start = self.start_ms;
        let final_date = Timestamp::from_millisecond(self.end_ms - 1)?
            .to_zoned(zone.clone())
            .date();
        while start < self.end_ms {
            let local = Timestamp::from_millisecond(start)?.to_zoned(zone.clone());
            let end = if local.date() == final_date {
                self.end_ms
            } else {
                local
                    .start_of_day()?
                    .tomorrow()?
                    .start_of_day()?
                    .timestamp()
                    .as_millisecond()
                    .min(self.end_ms)
            };
            if end <= start {
                return Err(Error::InvalidWindow);
            }
            buckets.push(DayBucket {
                date: local.date(),
                window: Self::new(start, end)?,
            });
            start = end;
        }
        Ok(buckets)
    }
}

/// Spread one half-open interval's already-measured duration across the ordered
/// buckets a window was split into, adding each overlap into `totals`.
///
/// `ends` holds each bucket's exclusive end in the same units as the interval,
/// in window order, and `totals` is the same length. The final bucket is open
/// above: an interval that reaches past the last end — a selected leap-second
/// event whose POSIX projection lands after the window's end — contributes the
/// remainder to the last reported day rather than disappearing. That is an
/// allocation of duration the metric already measured, not a new definition of
/// elapsed time, so the buckets still sum to the window's total.
///
/// Anything before the first bucket's start is not this window's time and is
/// dropped; the callers' queries already exclude it.
pub(crate) fn allocate(
    start_of_window: i128,
    ends: &[i128],
    interval: (i128, i128),
    totals: &mut [i128],
) -> Result<()> {
    if ends.len() != totals.len() || ends.is_empty() {
        return Err(Error::InvalidWindow);
    }
    let (start, end) = interval;
    let mut cursor = start.max(start_of_window);
    for (index, bucket_end) in ends.iter().enumerate() {
        if cursor >= end {
            break;
        }
        let last = index + 1 == ends.len();
        let stop = if last { end } else { end.min(*bucket_end) };
        if stop > cursor {
            totals[index] = totals[index]
                .checked_add(stop.checked_sub(cursor).ok_or(Error::CounterOverflow)?)
                .ok_or(Error::CounterOverflow)?;
            cursor = stop;
        }
    }
    Ok(())
}
