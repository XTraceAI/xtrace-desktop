//! The saved typing speed the M-15 typing estimate and the M-07 fallback read,
//! in whole words per minute at five characters per word. It lives in the
//! existing `settings` table under one key; an absent key reads as the default.
//! A stored value this build cannot read is an error, never a silent default.

use crate::{Error, Result, Store};
use rusqlite::{OptionalExtension, TransactionBehavior, params};

const SETTING: &str = "metrics.typing_speed_wpm";

/// Whole words per minute, `MIN_WPM..=MAX_WPM`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypingSpeed(u32);

impl TypingSpeed {
    /// The estimate's standard word: five characters, spaces included.
    pub const CHARACTERS_PER_WORD: u32 = 5;
    pub const MIN_WPM: u32 = 1;
    pub const MAX_WPM: u32 = 300;
    /// 40 WPM, the existing 200 characters per minute.
    pub const DEFAULT_WPM: u32 = 40;

    pub fn new(wpm: u32) -> Result<Self> {
        if (Self::MIN_WPM..=Self::MAX_WPM).contains(&wpm) {
            Ok(Self(wpm))
        } else {
            Err(Error::InvalidInput(
                "typing speed must be a whole number from 1 to 300 words per minute",
            ))
        }
    }
    pub fn wpm(self) -> u32 {
        self.0
    }
    pub fn characters_per_minute(self) -> u32 {
        self.0 * Self::CHARACTERS_PER_WORD
    }
}

impl Default for TypingSpeed {
    fn default() -> Self {
        Self(Self::DEFAULT_WPM)
    }
}

impl Store {
    pub fn typing_speed(&self) -> Result<TypingSpeed> {
        read(&self.connection)
    }

    /// Commits the new speed and returns the value read back inside the same
    /// transaction; nothing is written for a speed outside the bounds.
    pub fn set_typing_speed(&mut self, speed: TypingSpeed) -> Result<TypingSpeed> {
        let speed = TypingSpeed::new(speed.wpm())?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        transaction.execute(
            "INSERT INTO settings(key,value_json) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json",
            params![SETTING, speed.wpm().to_string()],
        )?;
        let saved = read(&transaction)?;
        transaction.commit()?;
        Ok(saved)
    }
}

fn read(connection: &rusqlite::Connection) -> Result<TypingSpeed> {
    let saved: Option<String> = connection
        .query_row(
            "SELECT value_json FROM settings WHERE key=?1",
            [SETTING],
            |row| row.get(0),
        )
        .optional()?;
    let Some(saved) = saved else {
        return Ok(TypingSpeed::default());
    };
    // Only a JSON integer is accepted: a decimal, string or out-of-range value
    // saved by anything else fails instead of changing the estimate.
    serde_json::from_str::<serde_json::Value>(&saved)?
        .as_u64()
        .and_then(|wpm| u32::try_from(wpm).ok())
        .and_then(|wpm| TypingSpeed::new(wpm).ok())
        .ok_or(Error::InvalidInput("stored typing speed is invalid"))
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
    fn absent_reads_the_default_forty_wpm_at_two_hundred_cpm() {
        let store = Store::open_in_memory().unwrap();
        let speed = store.typing_speed().unwrap();
        assert_eq!(speed, TypingSpeed::default());
        assert_eq!((speed.wpm(), speed.characters_per_minute()), (40, 200));
    }

    #[test]
    fn bounds_are_inclusive_whole_words() {
        for wpm in [1, 40, 300] {
            assert_eq!(TypingSpeed::new(wpm).unwrap().wpm(), wpm);
        }
        for wpm in [0, 301, u32::MAX] {
            assert!(TypingSpeed::new(wpm).is_err(), "{wpm}");
        }
        assert_eq!(TypingSpeed::new(300).unwrap().characters_per_minute(), 1500);
        assert_eq!(TypingSpeed::new(1).unwrap().characters_per_minute(), 5);
    }

    #[test]
    fn a_saved_speed_survives_reopening() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("typing.sqlite");
        let mut store = Store::open(&path).unwrap();
        let saved = store
            .set_typing_speed(TypingSpeed::new(80).unwrap())
            .unwrap();
        assert_eq!(saved.wpm(), 80);
        drop(store);
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.typing_speed().unwrap().wpm(), 80);
    }

    #[test]
    fn saving_the_default_again_replaces_the_row() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .set_typing_speed(TypingSpeed::new(300).unwrap())
            .unwrap();
        store.set_typing_speed(TypingSpeed::default()).unwrap();
        assert_eq!(store.typing_speed().unwrap(), TypingSpeed::default());
        let rows: i64 = store
            .connection
            .query_row(
                "SELECT count(*) FROM settings WHERE key=?1",
                [SETTING],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1);
    }

    #[test]
    fn stored_invalid_data_fails_visibly() {
        let mut store = Store::open_in_memory().unwrap();
        for value in [
            "0",
            "301",
            "-5",
            "40.5",
            "\"40\"",
            "null",
            "true",
            "[]",
            "{}",
            "18446744073709551616",
        ] {
            raw(&store, value);
            assert!(
                matches!(store.typing_speed(), Err(Error::InvalidInput(_))),
                "{value}"
            );
        }
        // A valid save repairs it.
        store
            .set_typing_speed(TypingSpeed::new(60).unwrap())
            .unwrap();
        assert_eq!(store.typing_speed().unwrap().wpm(), 60);
    }

    #[test]
    fn the_settings_table_refuses_non_json() {
        let store = Store::open_in_memory().unwrap();
        assert!(
            store
                .connection
                .execute(
                    "INSERT INTO settings(key,value_json) VALUES(?1,'not json')",
                    [SETTING],
                )
                .is_err()
        );
        assert_eq!(store.typing_speed().unwrap(), TypingSpeed::default());
    }

    #[test]
    fn the_key_is_separate_from_other_settings() {
        let mut store = Store::open_in_memory().unwrap();
        store
            .set_typing_speed(TypingSpeed::new(90).unwrap())
            .unwrap();
        assert_eq!(
            store.retention_mode().unwrap(),
            crate::retention::RetentionMode::MetadataOnly
        );
        assert_eq!(store.server_port().unwrap(), None);
    }
}
