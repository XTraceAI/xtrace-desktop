//! When the activity heartbeats are due. `daily_active` is sent at most once
//! per local calendar day; `hourly_active` only when the window or the
//! menu-bar popover is used, at most once an hour. A heartbeat counts as done
//! only once it was accepted for sending: one dropped while telemetry may not
//! send is not owed later.

/// The shortest time between two `hourly_active` events.
pub const HOURLY_DEBOUNCE_MS: i64 = 60 * 60 * 1000;

/// Whether `daily_active` is due on local day `today` (`YYYY-MM-DD`).
pub fn daily_due(last_accepted_day: Option<&str>, today: &str) -> bool {
    last_accepted_day != Some(today)
}

/// Whether `hourly_active` is due at `now_ms`. A clock that moved backwards
/// makes it due again rather than silencing it.
pub fn hourly_due(last_accepted_ms: Option<i64>, now_ms: i64) -> bool {
    last_accepted_ms.is_none_or(|last| now_ms < last || now_ms - last >= HOURLY_DEBOUNCE_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_is_due_once_per_local_day() {
        assert!(daily_due(None, "2026-10-12"));
        assert!(!daily_due(Some("2026-10-12"), "2026-10-12"));
        assert!(daily_due(Some("2026-10-12"), "2026-10-13"));
    }

    #[test]
    fn hourly_waits_a_full_hour() {
        let start = 1_800_000_000_000;
        assert!(hourly_due(None, start));
        assert!(!hourly_due(Some(start), start));
        assert!(!hourly_due(Some(start), start + HOURLY_DEBOUNCE_MS - 1));
        assert!(hourly_due(Some(start), start + HOURLY_DEBOUNCE_MS));
        assert!(hourly_due(Some(start), start - 1));
    }
}
