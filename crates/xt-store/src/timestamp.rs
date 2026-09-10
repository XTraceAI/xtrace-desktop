//! Precise native timestamp ordering, separate from the millisecond projection.

use crate::{Error, Result};

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct InstantKey {
    second: i64,
    leap_second: bool,
    fraction: String,
}

impl InstantKey {
    pub(crate) fn components(self) -> (i64, bool, String) {
        (self.second, self.leap_second, self.fraction)
    }
}

/// Chrono validates the RFC3339 spelling and normalizes the UTC second. Preserve
/// the original decimal fraction because Chrono truncates digits beyond nanos.
/// Trailing zeroes do not change an instant; remaining decimal strings compare
/// lexically even when they have different lengths. Leap seconds sort after the
/// ordinary :59 second and before the next UTC second.
pub(crate) fn parse(value: &str) -> Result<(InstantKey, i64)> {
    let parsed = chrono::DateTime::parse_from_rfc3339(value)
        .map_err(|_| Error::InvalidInput("timestamp must be RFC3339"))?;
    let fraction = value.split_once('.').map_or("", |(_, tail)| {
        let length = tail.bytes().take_while(u8::is_ascii_digit).count();
        tail[..length].trim_end_matches('0')
    });
    Ok((
        InstantKey {
            second: parsed.timestamp(),
            leap_second: parsed.timestamp_subsec_nanos() >= 1_000_000_000,
            fraction: fraction.to_owned(),
        },
        parsed.timestamp_millis(),
    ))
}
