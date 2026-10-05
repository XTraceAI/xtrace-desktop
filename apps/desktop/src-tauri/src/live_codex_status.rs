//! Ephemeral displayed-row status from Codex IPC and Claude's local registry.
//! Source content remains transient; no transcript persistence or logging,
//! process spawning, persistent storage or ownership RPC.
use crate::dto::{LiveSessionSnapshot, LiveSessionState, LiveSessionStatus};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "macos")]
mod claude;
#[cfg(unix)]
mod projection;
#[cfg(unix)]
mod protocol;
#[cfg(unix)]
mod socket;
#[cfg(all(test, unix))]
mod tests;

const MAX_TARGETS: usize = 16;
const MAX_VIEWS: usize = 4;
const MAX_ROWS: usize = 50;
const LEASE: Duration = Duration::from_secs(15);
static NEXT_VIEW: AtomicU64 = AtomicU64::new(0);
static BOOT_PREFIX: OnceLock<String> = OnceLock::new();
#[cfg(unix)]
const TICK: Duration = Duration::from_millis(50);

pub(crate) fn canonical_native(id: &str) -> Option<&str> {
    let native = id.strip_prefix("codex-")?;
    canonical_claude(native)
}

pub(crate) fn canonical_claude(id: &str) -> Option<&str> {
    let bytes = id.as_bytes();
    (bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                *b == b'-'
            } else {
                b.is_ascii_digit() || (b'a'..=b'f').contains(b)
            }
        }))
    .then_some(id)
}

pub(crate) fn validate_read(ids: &[String], view: Option<&str>) -> Result<(), &'static str> {
    if view.is_some_and(|view| !valid_view(view))
        || (view.is_none() && !ids.is_empty())
        || ids.len() > MAX_ROWS
        || ids.iter().any(|id| id.len() > 128)
    {
        return Err("invalid live status request");
    }
    Ok(())
}

fn valid_view(view: &str) -> bool {
    !view.is_empty()
        && view.len() <= 128
        && view
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_:".contains(&b))
}

struct View {
    targets: BTreeSet<String>,
    read_at: Instant,
    request_generation: u64,
}

struct Cached {
    generation: u64,
    status: LiveSessionState,
}

#[derive(Default)]
struct Hub {
    views: BTreeMap<String, View>,
    targets: BTreeMap<String, Cached>,
    generation: u64,
    closed: bool,
}

impl Hub {
    fn expire(&mut self, now: Instant) {
        self.views
            .retain(|_, view| now.duration_since(view.read_at) < LEASE);
        self.reconcile();
    }

    fn reconcile(&mut self) {
        let wanted: BTreeSet<String> = self
            .views
            .values()
            .flat_map(|v| v.targets.iter().cloned())
            .collect();
        self.targets.retain(|id, _| wanted.contains(id));
        for id in wanted {
            if !self.targets.contains_key(&id) {
                self.generation += 1;
                self.targets.insert(
                    id,
                    Cached {
                        generation: self.generation,
                        status: LiveSessionState::Unknown,
                    },
                );
            }
        }
    }
}

#[derive(Default)]
struct Shared {
    hub: Mutex<Hub>,
    wake: Condvar,
}

/// Exactly one lazy worker per app, independent of the database lock. The
/// renderer names indexed IDs and view leases, never a socket or home path.
pub struct LiveCodexStatus {
    home: Option<PathBuf>,
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
    claude_scan: Mutex<()>,
}

impl LiveCodexStatus {
    pub fn new(home: Option<&Path>) -> Self {
        Self {
            home: home.map(Path::to_owned),
            shared: Arc::default(),
            worker: Mutex::default(),
            claude_scan: Mutex::default(),
        }
    }

    pub(crate) fn read(
        &self,
        ids: &[String],
        view: Option<&str>,
        eligible: BTreeSet<String>,
    ) -> Result<LiveSessionSnapshot, &'static str> {
        self.read_with(ids, view, eligible, |home, targets, cancelled| {
            #[cfg(target_os = "macos")]
            return claude::scan(home, targets, cancelled).unwrap_or_default();
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (home, targets, cancelled);
                BTreeMap::new()
            }
        })
    }

    fn read_with(
        &self,
        ids: &[String],
        view: Option<&str>,
        eligible: BTreeSet<String>,
        scan: impl FnOnce(
            &Path,
            &BTreeSet<String>,
            &dyn Fn() -> bool,
        ) -> BTreeMap<String, LiveSessionState>,
    ) -> Result<LiveSessionSnapshot, &'static str> {
        validate_read(ids, view)?;
        let mut hub = self
            .shared
            .hub
            .lock()
            .map_err(|_| "live status is unavailable")?;
        hub.expire(Instant::now());
        if hub.closed {
            return Err("live status view was released");
        }
        let Some(view) = view else {
            if hub.views.len() >= MAX_VIEWS {
                return Err("too many live status views");
            }
            let sequence = NEXT_VIEW
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
                .map_err(|_| "live status view tokens exhausted")?;
            let prefix = BOOT_PREFIX.get_or_init(|| {
                format!(
                    "xtrace-{}-{}",
                    std::process::id(),
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .expect("system clock after epoch")
                        .as_nanos()
                )
            });
            let view_id = format!("{prefix}-{sequence}");
            hub.views.insert(
                view_id.clone(),
                View {
                    targets: BTreeSet::new(),
                    read_at: Instant::now(),
                    request_generation: 0,
                },
            );
            return Ok(LiveSessionSnapshot {
                view_id,
                states: Vec::new(),
            });
        };
        if !hub.views.contains_key(view) {
            return Err("live status view was released or expired");
        }
        let eligible = if self.home.is_some() && cfg!(unix) {
            eligible
                .into_iter()
                .filter(|id| {
                    ids.contains(id)
                        && (canonical_native(id).is_some()
                            || (cfg!(target_os = "macos") && canonical_claude(id).is_some()))
                })
                .collect()
        } else {
            BTreeSet::new()
        };
        let wanted: BTreeSet<&String> = hub
            .views
            .iter()
            .filter(|(id, _)| id.as_str() != view)
            .flat_map(|(_, v)| &v.targets)
            .chain(&eligible)
            .collect();
        if wanted.len() > MAX_TARGETS {
            return Err("live status supports at most 16 distinct sessions");
        }
        hub.generation = hub
            .generation
            .checked_add(1)
            .ok_or("live status generations exhausted")?;
        let request_generation = hub.generation;
        hub.views.insert(
            view.into(),
            View {
                targets: eligible,
                read_at: Instant::now(),
                request_generation,
            },
        );
        hub.reconcile();
        let generations: BTreeMap<_, _> = hub.views[view]
            .targets
            .iter()
            .map(|id| (id.clone(), hub.targets[id].generation))
            .collect();
        let claude_targets = generations
            .keys()
            .filter(|id| canonical_claude(id).is_some())
            .cloned()
            .collect::<BTreeSet<_>>();
        let needed = hub.targets.keys().any(|id| canonical_native(id).is_some());
        // Lock ordering is always hub -> worker. Shutdown does not join under
        // either lock, so the worker can finish unsubscribing and publishing.
        #[cfg(unix)]
        if needed {
            let mut worker = self
                .worker
                .lock()
                .map_err(|_| "live status is unavailable")?;
            if worker.is_none() {
                let shared = self.shared.clone();
                let home = self.home.clone().ok_or("live status is unavailable")?;
                *worker = Some(
                    std::thread::Builder::new()
                        .name("codex-status-observer".into())
                        .spawn(move || socket::run(home, shared))
                        .map_err(|_| "live status is unavailable")?,
                );
            }
        }
        #[cfg(not(unix))]
        let _ = needed;
        drop(hub);
        self.shared.wake.notify_all();
        // The command already runs on Tauri's blocking pool. No DB/hub lock,
        // extra worker, queued scans or cached Claude claim while doing I/O.
        let claude_states = if !claude_targets.is_empty() {
            match self.claude_scan.try_lock() {
                Ok(_guard) => scan(
                    self.home.as_deref().ok_or("live status is unavailable")?,
                    &claude_targets,
                    &|| !self.read_current(view, request_generation, &generations),
                ),
                Err(_) => BTreeMap::new(),
            }
        } else {
            BTreeMap::new()
        };
        let mut hub = self
            .shared
            .hub
            .lock()
            .map_err(|_| "live status is unavailable")?;
        hub.expire(Instant::now());
        if !Self::current(&hub, view, request_generation, &generations) {
            return Err("live status read was released, expired or replaced");
        }
        let mut distinct = BTreeSet::new();
        Ok(LiveSessionSnapshot {
            view_id: view.into(),
            states: ids
                .iter()
                .filter(|id| distinct.insert(id.as_str()))
                .map(|id| {
                    let status = if !generations.contains_key(id) {
                        LiveSessionState::Unknown
                    } else if canonical_claude(id).is_some() {
                        claude_states.get(id).copied().unwrap_or_default()
                    } else {
                        hub.targets
                            .get(id)
                            .map(|cache| cache.status)
                            .unwrap_or_default()
                    };
                    LiveSessionStatus {
                        id: id.clone(),
                        status,
                    }
                })
                .collect(),
        })
    }

    fn current(hub: &Hub, view: &str, request: u64, targets: &BTreeMap<String, u64>) -> bool {
        !hub.closed
            && hub
                .views
                .get(view)
                .is_some_and(|v| v.request_generation == request)
            && targets.iter().all(|(id, generation)| {
                hub.targets
                    .get(id)
                    .is_some_and(|cache| cache.generation == *generation)
            })
    }

    fn read_current(&self, view: &str, request: u64, targets: &BTreeMap<String, u64>) -> bool {
        let Ok(mut hub) = self.shared.hub.lock() else {
            return false;
        };
        hub.expire(Instant::now());
        Self::current(&hub, view, request, targets)
    }

    pub fn release(&self, view: &str) -> Result<(), &'static str> {
        if !valid_view(view) {
            return Err("invalid live status view");
        }
        let mut hub = self
            .shared
            .hub
            .lock()
            .map_err(|_| "live status is unavailable")?;
        hub.views.remove(view);
        hub.reconcile();
        drop(hub);
        self.shared.wake.notify_all();
        Ok(())
    }

    /// Check before metadata reads; read checks again after those reads finish.
    pub(crate) fn require_active(&self, view: &str) -> Result<(), &'static str> {
        let mut hub = self
            .shared
            .hub
            .lock()
            .map_err(|_| "live status is unavailable")?;
        hub.expire(Instant::now());
        if hub.closed || !hub.views.contains_key(view) {
            return Err("live status view was released or expired");
        }
        Ok(())
    }

    pub fn shutdown(&self) {
        {
            let mut hub = self
                .shared
                .hub
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            hub.closed = true;
            hub.views.clear();
            hub.targets.clear();
        }
        self.shared.wake.notify_all();
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(worker) = worker {
            let _ = worker.join();
        }
    }

    #[cfg(test)]
    pub(crate) fn requested_targets(&self) -> Vec<String> {
        self.shared
            .hub
            .lock()
            .unwrap()
            .targets
            .keys()
            .cloned()
            .collect()
    }
}

impl Drop for LiveCodexStatus {
    fn drop(&mut self) {
        self.shutdown();
    }
}
