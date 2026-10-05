use crate::{LoadedSession, Result, invalid, load::PrLinkInput};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use tempfile::{NamedTempFile, TempDir};
use xt_store::Store;

/// Own this value for the whole reader/server test. Dropping the Store before
/// the TempDir lets SQLite close and checkpoint before its files are removed.
pub struct TempDb {
    store: Store,
    path: PathBuf,
    directory: TempDir,
}

impl TempDb {
    /// Empty file-backed owner for schema and discovery tests that do not yet
    /// claim a populated fixture's product-rule acceptance.
    pub fn empty() -> Result<Self> {
        Self::build(&[], &[], false)
    }

    pub(crate) fn build(
        sessions: &[LoadedSession],
        pull_requests: &[PrLinkInput],
        keep_content: bool,
    ) -> Result<Self> {
        let directory = TempDir::new()?;
        let path = directory.path().join("fixture.sqlite");
        let mut store = Store::open(&path)?;
        if keep_content {
            // Content fixtures explicitly opt in; empty databases retain the production default.
            store.set_retention_mode(xt_store::retention::RetentionMode::FullContent)?;
        }
        for session in sessions {
            store.upsert_session(&session.metadata, keep_content)?;
            store.upsert_records(&session.metadata.session_id, &session.records, keep_content)?;
        }
        // Links only: the refresh-owned columns stay unset, as they are for a
        // pull request nothing has refreshed yet.
        for link in pull_requests {
            store.record_pr_link(&link.observation()?)?;
        }
        Ok(Self {
            store,
            path,
            directory,
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }
    pub fn store_mut(&mut self) -> &mut Store {
        &mut self.store
    }
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Only called on a newly built, privately owned database: no external
    /// connection has observed its path. SQLite checkpoints on the final close.
    pub(crate) fn write_to(self, destination: &Path) -> Result<()> {
        let Self {
            store,
            path,
            directory,
        } = self;
        drop(store);
        let sidecar = |path: &Path, suffix: &str| {
            let mut name = path.as_os_str().to_os_string();
            name.push(suffix);
            PathBuf::from(name)
        };
        if sidecar(&path, "-wal").try_exists()? {
            return Err(invalid(
                "fixture-db",
                "database did not checkpoint on close",
            ));
        }
        for suffix in ["-wal", "-shm", "-journal"] {
            match fs::symlink_metadata(sidecar(destination, suffix)) {
                Ok(_) => {
                    return Err(invalid(
                        destination.display().to_string(),
                        "refusing output with existing SQLite sidecars",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut output = NamedTempFile::new_in(parent)?;
        io::copy(&mut fs::File::open(path)?, output.as_file_mut())?;
        output.as_file().sync_all()?;
        output.persist_noclobber(destination).map_err(|e| e.error)?;
        drop(directory);
        Ok(())
    }
}
