//! The saved break length "your hours" read: the longest gap, in whole minutes,
//! between two messages a person sent that still counts as one stretch of
//! work. It lives in the existing `settings` table under one key; an absent key
//! reads as the default. A stored value this build cannot read is an error,
//! never a silent default.

use crate::{Error, Result, Store};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

const SETTING: &str = "metrics.human_break_minutes";

/// Whole minutes, `MIN_MINUTES..=MAX_MINUTES`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HumanBreak(u32);

impl HumanBreak {
    pub const MIN_MINUTES: u32 = 5;
    pub const MAX_MINUTES: u32 = 240;
    /// An hour: back-to-back half-hour meetings still leave moments with
    /// agents, while lunch and sleep are longer.
    pub const DEFAULT_MINUTES: u32 = 60;

    pub fn new(minutes: u32) -> Result<Self> {
        if (Self::MIN_MINUTES..=Self::MAX_MINUTES).contains(&minutes) {
            Ok(Self(minutes))
        } else {
            Err(Error::InvalidInput(
                "break length must be a whole number from 5 to 240 minutes",
            ))
        }
    }
    pub fn minutes(self) -> u32 {
        self.0
    }
}

impl Default for HumanBreak {
    fn default() -> Self {
        Self(Self::DEFAULT_MINUTES)
    }
}

impl Store {
    pub fn human_break(&self) -> Result<HumanBreak> {
        read(&self.connection)
    }

    /// Commits the new length and returns the value read back inside the same
    /// transaction; nothing is written for a length outside the bounds.
    pub fn set_human_break(&mut self, length: HumanBreak) -> Result<HumanBreak> {
        let length = HumanBreak::new(length.minutes())?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO settings(key,value_json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
            params![SETTING, length.minutes().to_string()],
        )?;
        let saved = read(&transaction)?;
        transaction.commit()?;
        Ok(saved)
    }
}

fn read(connection: &rusqlite::Connection) -> Result<HumanBreak> {
    let saved: Option<String> = connection
        .query_row(
            "SELECT value_json FROM settings WHERE key=?1",
            [SETTING],
            |row| row.get(0),
        )
        .optional()?;
    let Some(saved) = saved else {
        return Ok(HumanBreak::default());
    };
    // Only a JSON integer is accepted: a decimal, string or out-of-range value
    // saved by anything else fails instead of changing the hours.
    serde_json::from_str::<serde_json::Value>(&saved)?
        .as_u64()
        .and_then(|minutes| u32::try_from(minutes).ok())
        .and_then(|minutes| HumanBreak::new(minutes).ok())
        .ok_or(Error::InvalidInput("stored break length is invalid"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(store: &Store, value: &str) {
        store
            .connection
            .execute(
                "INSERT INTO settings(key,value_json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
                params![SETTING, value],
            )
            .unwrap();
    }

    #[test]
    fn absent_reads_the_default_hour() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.human_break().unwrap(), HumanBreak::default());
        assert_eq!(store.human_break().unwrap().minutes(), 60);
    }

    #[test]
    fn bounds_are_inclusive_whole_minutes() {
        for minutes in [5, 60, 240] {
            assert_eq!(HumanBreak::new(minutes).unwrap().minutes(), minutes);
        }
        for minutes in [0, 4, 241, u32::MAX] {
            assert!(HumanBreak::new(minutes).is_err(), "{minutes}");
        }
    }

    #[test]
    fn a_saved_length_survives_reopening() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("break.sqlite");
        let mut store = Store::open(&path).unwrap();
        let saved = store.set_human_break(HumanBreak::new(45).unwrap()).unwrap();
        assert_eq!(saved.minutes(), 45);
        drop(store);
        assert_eq!(
            Store::open(&path).unwrap().human_break().unwrap().minutes(),
            45
        );
    }

    #[test]
    fn stored_invalid_data_fails_visibly() {
        let mut store = Store::open_in_memory().unwrap();
        for value in ["0", "4", "241", "-5", "60.5", "\"60\"", "null", "[]"] {
            raw(&store, value);
            assert!(
                matches!(store.human_break(), Err(Error::InvalidInput(_))),
                "{value}"
            );
        }
        store.set_human_break(HumanBreak::new(90).unwrap()).unwrap();
        assert_eq!(store.human_break().unwrap().minutes(), 90);
    }

    #[test]
    fn the_key_is_separate_from_the_typing_speed() {
        let mut store = Store::open_in_memory().unwrap();
        store.set_human_break(HumanBreak::new(30).unwrap()).unwrap();
        assert_eq!(
            store.typing_speed().unwrap(),
            crate::typing_speed::TypingSpeed::default()
        );
        store
            .set_typing_speed(crate::typing_speed::TypingSpeed::new(80).unwrap())
            .unwrap();
        assert_eq!(store.human_break().unwrap().minutes(), 30);
    }
}
