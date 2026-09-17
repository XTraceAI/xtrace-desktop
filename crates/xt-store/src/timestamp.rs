//! Precise native timestamp ordering, separate from the millisecond projection.

use crate::{Error, Result};

#[derive(PartialEq, Eq, PartialOrd, Ord)]
pub struct InstantKey {
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
pub fn parse(value: &str) -> Result<(InstantKey, i64)> {
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

/// Register the precise timestamp comparator required by the response-usage view.
/// Raw SQLite readers must register it before querying that view. No database
/// state is read or written; invalid stored timestamps return a SQLite error.
pub fn register_sqlite(connection: &rusqlite::Connection) -> rusqlite::Result<()> {
    use rusqlite::functions::FunctionFlags;
    connection.create_scalar_function(
        "xt_timestamp_cmp",
        2,
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |context| {
            let instant = |index| -> rusqlite::Result<Option<InstantKey>> {
                context
                    .get::<Option<String>>(index)?
                    .map(|value| {
                        parse(&value)
                            .map(|(key, _)| key)
                            .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))
                    })
                    .transpose()
            };
            Ok(match instant(0)?.cmp(&instant(1)?) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            })
        },
    )
}
