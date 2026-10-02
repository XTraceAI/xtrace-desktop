//! Serialized IPC and fixture contracts. Integer constructors enforce JSON precision.
pub use crate::dashboard_dto::*;
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

/// The native index the app keeps over the local Claude, Codex and Cursor
/// history: its lifecycle phase, whether live changes reach it, what the
/// reader hosts need, and each host's last scan. Paths of native sources
/// stay out of it; they are local index metadata.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct NativeIndexStatus {
    pub phase: NativeIndexPhase,
    pub freshness: NativeFreshness,
    pub python: PythonRuntime,
    pub readers: ReaderBundle,
    pub hosts: Vec<NativeHostStatus>,
    /// Reconciliations completed: the initial scan, then one per change burst.
    pub reconciles: u32,
    /// Claude transcripts read so far by the initial scan (progress while scanning).
    pub files_scanned: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum NativeIndexPhase {
    /// No index runs in this process: fixture mode, or a data directory the
    /// index must not use.
    Disabled { reason: String },
    /// The initial scan (and the reconciliation of changes made during it).
    Scanning,
    /// Ready: the initial scan is done and live changes are reconciled.
    Ready,
    /// Stopped at shutdown, or the worker ended on its own.
    Stopped,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "freshness", rename_all = "snake_case")]
pub enum NativeFreshness {
    /// Not known yet: the watcher is not registered.
    Unknown,
    Live,
    /// The index reflects the last scan only.
    Degraded {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PythonRuntime {
    /// Discovery runs beside the initial scan; the reader hosts' own scans
    /// resolve the interpreter themselves meanwhile.
    Resolving,
    /// The interpreter the reader hosts run with, as found at startup.
    Available { path: String },
    /// Claude indexing continues; Codex and Cursor report the missing runtime.
    Missing { reason: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ReaderBundle {
    /// The bundled reader sources are exactly the pinned producer's objects.
    Verified {
        commit: String,
        plugin_version: String,
    },
    Unavailable {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct NativeHostStatus {
    /// `claude`, `codex` or `cursor`.
    pub host: String,
    pub state: NativeHostState,
    pub detail: Option<String>,
    pub sessions_imported: u32,
    pub sessions_partial: u32,
    pub sessions_skipped: u32,
    pub records_new: u32,
    pub records_enriched: u32,
    pub diagnostics: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum NativeHostState {
    /// Not scanned yet in this process.
    Pending,
    Complete,
    Incomplete,
    MissingSource,
    MissingRuntime,
    PinMismatch,
    ReaderFailed,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Serialize, TS)]
pub struct FixtureExport {
    pub app_info: AppInfo,
    pub db_counts: DbCounts,
    pub native_index: NativeIndexStatus,
    pub sessions: SessionPage,
    pub dashboards: Vec<DashboardMetrics>,
}

pub fn export_types(directory: impl AsRef<std::path::Path>) -> Result<(), ts_rs::ExportError> {
    TokensByHost::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
    SessionPage::export_all(&ts_rs::Config::new().with_out_dir(directory.as_ref()))?;
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

/// Initial metadata browser. Product time/token metrics are separate projections.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct SessionRow {
    pub id: String,
    pub host: String,
    pub repo: Option<String>,
    pub branch: Option<String>,
    pub model: Option<String>,
    pub first_ts: Option<String>,
    #[ts(type = "number")]
    pub record_count: u64,
    pub has_conflict: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
pub struct SessionPage {
    pub rows: Vec<SessionRow>,
    pub next: Option<String>,
}

pub fn session_page(
    store: &xt_store::Store,
    search: &str,
    host: Option<&str>,
    after: Option<&str>,
) -> xt_store::Result<SessionPage> {
    if after.is_some_and(|s| s.len() > 4096) {
        return Err(xt_store::Error::InvalidInput("invalid session cursor"));
    }
    let cursor = after
        .map(serde_json::from_str::<xt_store::session_list::SessionCursor>)
        .transpose()
        .map_err(|_| xt_store::Error::InvalidInput("invalid session cursor"))?;
    let mut rows = store.sessions_page(search, host, cursor.as_ref())?;
    let next = if rows.len() > 50 {
        rows.truncate(50);
        Some(serde_json::to_string(&rows.last().unwrap().cursor)?)
    } else {
        None
    };
    if rows.iter().any(|r| r.record_count >= 1u64 << 53) {
        return Err(xt_store::Error::InvalidInput(
            "record count exceeds JSON precision",
        ));
    }
    Ok(SessionPage {
        next,
        rows: rows
            .into_iter()
            .map(|r| SessionRow {
                id: r.id,
                host: r.host,
                repo: r.repo,
                branch: r.branch,
                model: r.model,
                first_ts: r.first_ts,
                record_count: r.record_count,
                has_conflict: r.has_conflict,
            })
            .collect(),
    })
}
