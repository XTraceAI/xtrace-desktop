use crate::{Error, Result, Store};
use rusqlite::OptionalExtension;

impl Store {
    /// Read the configured loopback port; absence uses the server default.
    /// Zero is reserved for explicit ephemeral test-port selection.
    pub fn server_port(&self) -> Result<Option<u16>> {
        let raw: Option<String> = self
            .connection
            .query_row(
                "SELECT value_json FROM settings WHERE key='server.port'",
                [],
                |row| row.get(0),
            )
            .optional()?;
        raw.map(|raw| {
            let value: serde_json::Value = serde_json::from_str(&raw)?;
            value
                .as_u64()
                .and_then(|value| u16::try_from(value).ok())
                .filter(|value| *value != 0)
                .ok_or(Error::InvalidInput(
                    "server.port must be an integer between 1 and 65535",
                ))
        })
        .transpose()
    }
}
