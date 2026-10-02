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
