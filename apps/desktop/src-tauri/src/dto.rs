//! Serialized IPC and fixture contracts. Integer constructors enforce JSON precision.
use serde::Serialize;
use ts_rs::TS;
use xt_store::StoreCounts;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    pub data_dir: String,
    pub fixture: Option<String>,
    pub schema_version: u32,
    pub listening: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct DbCounts {
    #[ts(type = "number")]
    sessions: u64,
    #[ts(type = "number")]
    records: u64,
    #[ts(type = "number")]
    usage: u64,
}

impl TryFrom<StoreCounts> for DbCounts {
    type Error = &'static str;
    fn try_from(value: StoreCounts) -> Result<Self, Self::Error> {
        if [value.sessions, value.records, value.usage_rows]
            .iter()
            .any(|value| *value >= 1_u64 << 53)
        {
            return Err("database count exceeds the exact JSON integer range");
        }
        Ok(Self {
            sessions: value.sessions,
            records: value.records,
            usage: value.usage_rows,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct FixtureExport {
    pub app_info: AppInfo,
    pub db_counts: DbCounts,
}

pub fn export_types(directory: impl AsRef<std::path::Path>) -> Result<(), ts_rs::ExportError> {
    FixtureExport::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_counts_preserve_json_precision() {
        let maximum = (1_u64 << 53) - 1;
        let accepted = DbCounts::try_from(StoreCounts {
            sessions: maximum,
            records: 0,
            usage_rows: 0,
        })
        .unwrap();
        assert_eq!(serde_json::to_value(accepted).unwrap()["sessions"], maximum);
        for value in [1_u64 << 53, u64::MAX] {
            for counts in [
                StoreCounts {
                    sessions: value,
                    ..Default::default()
                },
                StoreCounts {
                    records: value,
                    ..Default::default()
                },
                StoreCounts {
                    usage_rows: value,
                    ..Default::default()
                },
            ] {
                assert!(DbCounts::try_from(counts).is_err());
            }
        }
    }
}
