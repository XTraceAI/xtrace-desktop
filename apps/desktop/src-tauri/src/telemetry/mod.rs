//! Product analytics: the consent gate and the PostHog sender.
//! `TELEMETRY.md` at the repository root lists every event and
//! property; a test below keeps it and [`events`] the same.
//!
//! The store is the record of the two consent facts. The running app keeps a
//! copy in a [`consent::Gate`] that the sender checks before every request:
//! loaded at startup, then changed only by the writes below, each applied
//! while its save still holds the store.
pub mod consent;
pub mod events;
pub mod heartbeat;
pub mod posthog;
pub mod sender;

use crate::state::{AppState, StateError};
use consent::{Consent, Gate, TelemetryState};
use std::sync::Arc;

/// The bundle identifier of the installed app. Only it sends: test copies
/// run under identifiers of their own (for example `ai.xtrace.app.test`).
pub const INSTALLED_IDENTIFIER: &str = "ai.xtrace.app";

/// The one rule for whether this launch may have a sender, which the state
/// reports as `configured`: a project key compiled into the build, the
/// installed app's identifier, and `own_live_data`: live data (not a
/// disposable fixture) in the app's own data folder (see
/// [`uses_default_data_folder`]). A run on another folder would otherwise
/// make a new install ID, and count as another user, for every folder.
pub fn sender_config(
    build: Option<posthog::PostHogConfig>,
    own_live_data: bool,
    identifier: &str,
) -> Option<posthog::PostHogConfig> {
    build.filter(|_| own_live_data && identifier == INSTALLED_IDENTIFIER)
}

/// Whether this launch uses the app's own data folder, by the rule startup
/// already uses to choose it ([`crate::state::StartupOptions::data_folder`]):
/// no fixture, and no `XTRACE_DATA_DIR` or one naming that same folder.
/// `default_dir` is the app's data folder, as startup resolves it.
pub fn uses_default_data_folder(
    options: &crate::state::StartupOptions,
    default_dir: impl Fn() -> Result<std::path::PathBuf, StateError>,
) -> bool {
    use crate::state::DataFolder;
    match options.data_folder(&default_dir) {
        Ok(DataFolder::Default(_)) => true,
        Ok(DataFolder::Chosen(chosen)) => default_dir().is_ok_and(|default| {
            chosen == default
                || matches!(
                    (chosen.canonicalize(), default.canonicalize()),
                    (Ok(a), Ok(b)) if a == b
                )
        }),
        Ok(DataFolder::Fixture) | Err(_) => false,
    }
}

/// The running telemetry service. Without a sender (see [`sender_config`],
/// or no system `curl`) it still keeps the preference and the disclosure,
/// and never makes a request.
pub struct Telemetry {
    gate: Arc<Gate>,
    sender: Option<sender::Sender>,
}

impl Telemetry {
    pub fn without_sender(consent: Consent, env_off: bool) -> Self {
        Self {
            gate: Arc::new(Gate::new(consent, env_off)),
            sender: None,
        }
    }

    /// The service for one launch: a sender only when `config` is present
    /// (from [`sender_config`]) and `transport` can make one for it.
    pub fn for_launch(
        consent: Consent,
        env_off: bool,
        config: Option<posthog::PostHogConfig>,
        transport: impl FnOnce(&posthog::PostHogConfig) -> Option<Arc<dyn sender::Transport>>,
        store: Arc<dyn sender::TelemetryStore>,
        clock: Arc<dyn sender::Clock>,
    ) -> Self {
        match config.and_then(|config| transport(&config).map(|t| (config, t))) {
            Some((config, transport)) => {
                Self::start(consent, env_off, config, transport, store, clock)
            }
            None => Self::without_sender(consent, env_off),
        }
    }

    /// Starts the sender; nothing is sent until [`Self::opened`] or
    /// [`Self::activity`], and only while the gate allows it.
    pub fn start(
        consent: Consent,
        env_off: bool,
        config: posthog::PostHogConfig,
        transport: Arc<dyn sender::Transport>,
        store: Arc<dyn sender::TelemetryStore>,
        clock: Arc<dyn sender::Clock>,
    ) -> Self {
        let gate = Arc::new(Gate::new(consent, env_off));
        let sender = sender::Sender::start(gate.clone(), config, transport, store, clock);
        Self {
            gate,
            sender: Some(sender),
        }
    }

    /// The state this launch runs with. It reads nothing and changes nothing.
    pub fn state(&self) -> TelemetryState {
        let (consent, off_unsaved) = self.gate.snapshot();
        TelemetryState::decide(
            consent,
            self.gate.env_off(),
            self.sender.is_some(),
            off_unsaved,
        )
    }

    /// Drops queued and in-flight work once the gate is closed. Called after
    /// the store is released: dropping work writes the ledger.
    fn close_if_closed(&self) {
        if !self.gate.allows()
            && let Some(sender) = &self.sender
        {
            sender.close();
        }
    }

    /// Saves the preference. `save` writes it and passes what it read back to
    /// its `apply` argument before releasing the store. Turning off takes
    /// effect before saving and holds for this launch until telemetry is
    /// turned on again, so a failed save still stops sending. Refused under
    /// the environment override, before `save` is called.
    pub fn set_preference(
        &self,
        on: bool,
        save: impl FnOnce(&dyn Fn(Consent)) -> Result<(), StateError>,
    ) -> Result<TelemetryState, StateError> {
        if self.gate.env_off() {
            return Err(StateError::TelemetryEnvironmentOff);
        }
        let result = if on {
            save(&|saved| self.gate.turn_on(saved))
        } else {
            self.gate.hold_off();
            self.close_if_closed();
            save(&|saved| self.gate.set(saved))
        };
        self.close_if_closed();
        result?;
        Ok(self.state())
    }

    /// Records that the disclosure was visibly shown; `save` works as in
    /// [`Self::set_preference`]. Once that makes sending possible, the use
    /// that showed it counts as activity: only heartbeats made from now on can
    /// be sent, never anything from before.
    pub fn disclosure_shown(
        &self,
        save: impl FnOnce(&dyn Fn(Consent)) -> Result<(), StateError>,
    ) -> Result<TelemetryState, StateError> {
        save(&|saved| self.gate.set(saved))?;
        if self.gate.allows() {
            self.activity();
        } else {
            self.close_if_closed();
        }
        Ok(self.state())
    }

    /// The app finished starting.
    pub fn opened(&self) {
        if let Some(sender) = &self.sender {
            sender.opened();
        }
    }

    /// The main window or the menu-bar popover was used.
    pub fn activity(&self) {
        if let Some(sender) = &self.sender {
            sender.activity();
        }
    }

    /// Stops the sender before the store closes.
    pub fn shutdown(&self) {
        if let Some(sender) = &self.sender {
            sender.shutdown();
        }
    }
}

impl AppState {
    pub fn telemetry_consent(&self) -> Result<Consent, StateError> {
        self.with_store(|store| Ok(store.telemetry_settings()?.into()))
    }
    /// Saves the preference and hands what was read back to `apply` while
    /// the store is still held, so concurrent writes apply in save order.
    pub fn set_telemetry_enabled(
        &self,
        on: bool,
        apply: &dyn Fn(Consent),
    ) -> Result<(), StateError> {
        self.with_store(|store| {
            apply(store.set_telemetry_enabled(on)?.into());
            Ok(())
        })
    }
    /// Like [`Self::set_telemetry_enabled`], for the disclosure.
    pub fn set_telemetry_disclosure_shown(
        &self,
        apply: &dyn Fn(Consent),
    ) -> Result<(), StateError> {
        self.with_store(|store| {
            apply(store.set_telemetry_disclosure_shown()?.into());
            Ok(())
        })
    }
}

/// The event list published with this build's release tag, so an older
/// release never shows a newer build's list. A compiled constant: no URL
/// crosses IPC. A local build whose version has no tag with this file yet
/// opens a page that does not exist until the next version.
pub const TELEMETRY_DOC_URL: &str = concat!(
    "https://github.com/XTraceAI/xtrace-desktop/blob/v",
    env!("CARGO_PKG_VERSION"),
    "/TELEMETRY.md"
);

/// Hands the fixed event list page to the system browser; the app window
/// never navigates.
pub fn open_telemetry_doc() -> Result<(), StateError> {
    let status = telemetry_doc_command()?
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|_| StateError::TelemetryDocUnavailable)?;
    if status.success() {
        Ok(())
    } else {
        Err(StateError::TelemetryDocUnavailable)
    }
}

/// An absolute launcher path, so the search path cannot substitute another.
#[cfg(target_os = "macos")]
fn telemetry_doc_command() -> Result<std::process::Command, StateError> {
    let mut command = std::process::Command::new("/usr/bin/open");
    command.arg(TELEMETRY_DOC_URL);
    Ok(command)
}
#[cfg(not(target_os = "macos"))]
fn telemetry_doc_command() -> Result<std::process::Command, StateError> {
    Err(StateError::TelemetryDocUnavailable)
}

/// A random version-4 UUID from the system's random source, for the
/// anonymous install ID. None when no random bytes can be read.
pub fn random_uuid() -> Option<String> {
    use std::io::Read;
    let mut bytes = [0_u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .ok()?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Some(format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    ))
}

/// The sender's store calls through the app's managed state. Each call
/// fails closed once the state is gone or closed.
pub struct AppStore<R: tauri::Runtime>(pub tauri::AppHandle<R>);

impl<R: tauri::Runtime> AppStore<R> {
    fn with<T>(&self, call: impl FnOnce(&mut xt_store::Store) -> xt_store::Result<T>) -> Option<T> {
        use tauri::Manager;
        let state = self.0.try_state::<AppState>()?;
        state.with_store(|store| Ok(call(store)?)).ok()
    }
}

impl<R: tauri::Runtime> sender::TelemetryStore for AppStore<R> {
    fn hosts_detected(&self) -> Option<Vec<events::DetectedHost>> {
        let hosts = self.with(|store| store.indexed_hosts())?;
        Some(
            hosts
                .into_iter()
                .filter_map(events::DetectedHost::from_store)
                .collect(),
        )
    }
    fn distinct_id(&self) -> Option<String> {
        let generated = random_uuid()?;
        self.with(|store| store.telemetry_distinct_id(|| generated))
    }
    fn record_pending(&self, at_ms: i64, event: &str, properties_json: &str) -> Option<i64> {
        self.with(|store| store.telemetry_record_pending(at_ms, event, properties_json))
    }
    fn mark(&self, id: i64, status: xt_store::telemetry::DeliveryStatus) {
        self.with(|store| store.telemetry_mark(id, status));
    }
    fn fail_pending(&self) {
        self.with(|store| store.telemetry_fail_pending());
    }
    fn daily_active_day(&self) -> Option<Option<String>> {
        self.with(|store| store.telemetry_daily_active_day())
    }
    fn set_daily_active_day(&self, day: &str) -> bool {
        self.with(|store| store.set_telemetry_daily_active_day(day))
            .is_some()
    }
    fn hourly_active_at(&self) -> Option<Option<i64>> {
        self.with(|store| store.telemetry_hourly_active_at())
    }
    fn set_hourly_active_at(&self, at_ms: i64) -> bool {
        self.with(|store| store.set_telemetry_hourly_active_at(at_ms))
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::consent::TelemetryStatus;
    use super::events::{COMMON_PROPERTIES, DetectedHost, TelemetryEvent};
    use super::sender::tests::{MockTransport, ON, TestStore, rig_with};
    use super::*;
    use std::collections::BTreeSet;
    use xt_store::{SessionMeta, SessionSource, Store};

    fn store_with_consent(path: &std::path::Path) -> (Store, Consent) {
        let store = Store::open(path).unwrap();
        let consent = store.telemetry_settings().unwrap().into();
        (store, consent)
    }

    #[test]
    fn consent_persists_across_a_reopened_store_and_turn_off_survives_restart() {
        let directory = tempfile::TempDir::new().unwrap();
        let path = directory.path().join("xtrace.db");
        let (store, fresh) = store_with_consent(&path);
        assert_eq!(fresh, Consent::FRESH);
        let store = std::sync::Mutex::new(store);
        let telemetry = Telemetry::without_sender(fresh, false);
        let shown = telemetry
            .disclosure_shown(|apply| {
                apply(
                    store
                        .lock()
                        .unwrap()
                        .set_telemetry_disclosure_shown()?
                        .into(),
                );
                Ok(())
            })
            .unwrap();
        assert!(shown.can_send);
        let off = telemetry
            .set_preference(false, |apply| {
                apply(store.lock().unwrap().set_telemetry_enabled(false)?.into());
                Ok(())
            })
            .unwrap();
        assert!(!off.can_send && !off.preference_on);
        drop(store);
        // Restart: the same facts are read back.
        let (_store, saved) = store_with_consent(&path);
        let state = Telemetry::without_sender(saved, false).state();
        assert!(!state.preference_on && state.disclosure_shown && !state.can_send);
    }

    /// The re-read after a failed Turn off, and any later write that
    /// read "on", leave telemetry off; only turning it on again reopens it.
    #[test]
    fn a_failed_turn_off_holds_until_telemetry_is_turned_on_again() {
        let telemetry = Telemetry::without_sender(ON, false);
        let failed = telemetry.set_preference(false, |_| Err(StateError::Closed));
        assert!(failed.is_err());
        // The renderer re-reads after the error; the store still says on.
        let state = telemetry.state();
        assert!(!telemetry.gate.allows());
        assert!(!state.can_send && !state.preference_on && !state.enabled);
        // A disclosure write that read "on" from the store does not reopen it.
        telemetry
            .disclosure_shown(|apply| {
                apply(ON);
                Ok(())
            })
            .unwrap();
        assert!(!telemetry.gate.allows());
        // A failed Turn on keeps it off too.
        assert!(
            telemetry
                .set_preference(true, |_| Err(StateError::Closed))
                .is_err()
        );
        assert!(!telemetry.gate.allows());
        let on = telemetry
            .set_preference(true, |apply| {
                apply(ON);
                Ok(())
            })
            .unwrap();
        assert!(on.can_send && telemetry.gate.allows());
    }

    /// A Turn off that could not be saved is reported as off only until
    /// the app restarts, never as the saved choice; a saved Turn off or Turn
    /// on ends that.
    #[test]
    fn a_turn_off_that_was_not_saved_is_reported_as_off_until_restart() {
        let mut store = Store::open_in_memory().unwrap();
        let shown: Consent = store.set_telemetry_disclosure_shown().unwrap().into();
        let store = &std::sync::Mutex::new(store);
        let telemetry = Telemetry::without_sender(shown, false);
        let save = |on: bool, fail: bool| {
            move |apply: &dyn Fn(Consent)| {
                if fail {
                    return Err(StateError::Closed);
                }
                apply(store.lock().unwrap().set_telemetry_enabled(on)?.into());
                Ok(())
            }
        };
        assert!(telemetry.set_preference(false, save(false, true)).is_err());
        let state = telemetry.state();
        assert!(store.lock().unwrap().telemetry_settings().unwrap().enabled);
        assert_eq!(state.status, TelemetryStatus::OffUntilRestart);
        assert!(!state.can_send && !state.enabled && !state.show_disclosure);
        // A Turn on that also fails leaves the same truthful state.
        assert!(telemetry.set_preference(true, save(true, true)).is_err());
        assert_eq!(telemetry.state().status, TelemetryStatus::OffUntilRestart);
        // Saving Turn off makes it the saved choice.
        let off = telemetry.set_preference(false, save(false, false)).unwrap();
        assert!(!store.lock().unwrap().telemetry_settings().unwrap().enabled);
        // (This service has no sender, so the saved off reads as not configured.)
        assert_eq!(off.status, TelemetryStatus::NotConfigured);
        assert!(!off.preference_on);
        // A failed Turn off, then a saved Turn on, also ends it.
        let on = telemetry.set_preference(true, save(true, false)).unwrap();
        assert!(on.preference_on && on.can_send);
        assert!(telemetry.set_preference(false, save(false, true)).is_err());
        assert_eq!(telemetry.state().status, TelemetryStatus::OffUntilRestart);
        let on = telemetry.set_preference(true, save(true, false)).unwrap();
        assert_ne!(on.status, TelemetryStatus::OffUntilRestart);
        assert!(on.preference_on && on.can_send);
    }

    /// A write that read "on" from the store before Turn off cannot
    /// apply it after Turn off. Each write applies what it read while it still
    /// holds the store, and Turn off takes effect before it waits for it.
    #[test]
    fn a_write_that_read_on_before_turn_off_cannot_reopen_the_gate() {
        use std::sync::{Mutex, mpsc};
        let store = Arc::new(Mutex::new(Store::open_in_memory().unwrap()));
        let telemetry = Arc::new(Telemetry::without_sender(Consent::FRESH, false));
        let (inside, holding) = mpsc::channel();
        let (off_started, wait_for_off) = mpsc::channel();
        let writer = {
            let (store, telemetry) = (store.clone(), telemetry.clone());
            std::thread::spawn(move || {
                telemetry
                    .disclosure_shown(|apply| {
                        let mut store = store.lock().unwrap();
                        let saved: Consent = store.set_telemetry_disclosure_shown()?.into();
                        assert!(saved.preference_on, "this write read \"on\"");
                        inside.send(()).unwrap();
                        // The user turns telemetry off while this write holds the store.
                        wait_for_off.recv().unwrap();
                        apply(saved);
                        Ok(())
                    })
                    .unwrap()
            })
        };
        holding.recv().unwrap();
        let off = telemetry
            .set_preference(false, |apply| {
                off_started.send(()).unwrap();
                let mut store = store.lock().unwrap();
                apply(store.set_telemetry_enabled(false)?.into());
                Ok(())
            })
            .unwrap();
        let shown = writer.join().unwrap();
        assert!(!shown.can_send, "the earlier write applied while off held");
        assert!(!off.can_send && !telemetry.gate.allows());
        let saved = store.lock().unwrap().telemetry_settings().unwrap();
        assert!(!saved.enabled && saved.disclosure_shown);
        let state = telemetry.state();
        assert_eq!(
            (state.preference_on, state.disclosure_shown),
            (saved.enabled, saved.disclosure_shown),
            "the running gate equals the store"
        );
    }

    #[test]
    fn the_environment_override_refuses_changes_before_storage_is_touched() {
        let telemetry = Telemetry::without_sender(ON, true);
        for on in [true, false] {
            let result = telemetry.set_preference(on, |_| unreachable!("nothing is saved"));
            assert!(matches!(result, Err(StateError::TelemetryEnvironmentOff)));
        }
        let state = telemetry.state();
        assert!(!state.can_send && !state.can_change);
        assert_eq!(state.status, TelemetryStatus::EnvironmentOff);
    }

    #[test]
    fn a_build_without_a_key_reports_not_configured_and_asks_for_no_disclosure() {
        let state = Telemetry::without_sender(Consent::FRESH, false).state();
        assert!(!state.configured && !state.show_disclosure);
        assert_eq!(state.status, TelemetryStatus::NotConfigured);
    }

    fn launch(identifier: &str, own_live_data: bool) -> (Telemetry, Arc<MockTransport>, bool) {
        let transport = MockTransport::answering(&[]);
        let mut made = false;
        let config = sender_config(
            posthog::PostHogConfig::new(Some(posthog::DEFAULT_KEY), None),
            own_live_data,
            identifier,
        );
        let telemetry = Telemetry::for_launch(
            ON,
            false,
            config,
            |_| {
                made = true;
                Some(transport.clone() as Arc<dyn sender::Transport>)
            },
            TestStore::new(Store::open_in_memory().unwrap()),
            Arc::new(sender::SystemClock),
        );
        (telemetry, transport, made)
    }

    #[test]
    fn only_the_installed_identifier_with_live_data_gets_a_sender() {
        for (identifier, live) in [
            ("ai.xtrace.app.test", true),
            ("ai.xtrace.desktop", true),
            ("ai.xtrace.app", false),
        ] {
            let (telemetry, transport, made) = launch(identifier, live);
            let state = telemetry.state();
            assert!(!made, "{identifier}");
            assert!(!state.configured);
            assert_eq!(state.status, TelemetryStatus::NotConfigured, "{identifier}");
            telemetry.opened();
            telemetry.activity();
            telemetry.shutdown();
            assert_eq!(transport.requests(), 0);
        }
        let (telemetry, _, made) = launch(INSTALLED_IDENTIFIER, true);
        let state = telemetry.state();
        telemetry.shutdown();
        assert!(made && state.configured);
        assert_eq!(state.status, TelemetryStatus::Sending);
    }

    /// The real identifier with live data in another folder (a test run
    /// with `XTRACE_DATA_DIR`) gets no sender.
    #[test]
    fn a_custom_data_folder_gets_no_sender_even_with_the_installed_identifier() {
        use crate::state::StartupOptions;
        let directory = tempfile::TempDir::new().unwrap();
        let (own, custom) = (
            directory.path().join("own"),
            directory.path().join("custom"),
        );
        std::fs::create_dir_all(&own).unwrap();
        std::fs::create_dir_all(&custom).unwrap();
        let default_dir = || Ok(own.clone());
        let chosen = |path: std::path::PathBuf| StartupOptions {
            data_dir: Some(path),
            ..Default::default()
        };
        assert!(uses_default_data_folder(
            &StartupOptions::default(),
            default_dir
        ));
        assert!(uses_default_data_folder(&chosen(own.clone()), default_dir));
        // The same folder spelled another way is still the app's own.
        assert!(uses_default_data_folder(
            &chosen(directory.path().join("custom/../own")),
            default_dir
        ));
        assert!(!uses_default_data_folder(
            &chosen(custom.clone()),
            default_dir
        ));
        let fixture = StartupOptions {
            fixture: Some("F1".into()),
            ..Default::default()
        };
        assert!(!uses_default_data_folder(&fixture, default_dir));
        // Unresolvable own folder: not known to be the same, so no sender.
        assert!(!uses_default_data_folder(&chosen(custom.clone()), || {
            Err(StateError::InvalidOption)
        }));
        // The launch decision for a custom folder with the real identifier.
        let own_live_data = uses_default_data_folder(&chosen(custom), default_dir);
        let (telemetry, transport, made) = launch(INSTALLED_IDENTIFIER, own_live_data);
        let state = telemetry.state();
        telemetry.opened();
        telemetry.activity();
        telemetry.shutdown();
        assert!(!made && !state.configured);
        assert_eq!(state.status, TelemetryStatus::NotConfigured);
        assert_eq!(transport.requests(), 0);
    }

    #[test]
    fn the_install_id_is_a_random_version_four_uuid() {
        let (a, b) = (random_uuid().unwrap(), random_uuid().unwrap());
        assert_ne!(a, b);
        assert_eq!(a.len(), 36);
        assert_eq!(&a[14..15], "4");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"));
    }

    /// Strings in the store and the surrounding state that must never reach
    /// a payload or the ledger.
    const SENTINELS: [&str; 7] = [
        "/Users/sentinel-user/projects/sentinel-repo",
        "sentinel-repo-name",
        "SENTINEL PROMPT TEXT",
        "https://github.com/sentinel-org/sentinel-repo/pull/4242",
        "987654321",
        "sentinel-raw-surface",
        "sentinel-host",
    ];

    fn store_full_of_sentinels() -> Store {
        let mut store = Store::open_in_memory().unwrap();
        store
            .set_retention_mode(xt_store::retention::RetentionMode::FullContent)
            .unwrap();
        for (id, platform) in [("s1", "claude"), ("s2", "codex"), ("s3", SENTINELS[6])] {
            let mut meta = SessionMeta::new(id, platform, SessionSource::Plugin);
            meta.cwd = Some(SENTINELS[0].into());
            meta.git_branch = Some(SENTINELS[1].into());
            meta.title = Some(SENTINELS[2].into());
            meta.surface = Some(SENTINELS[5].into());
            meta.native_session_id = Some(format!("{} {}", SENTINELS[3], SENTINELS[4]));
            store.upsert_session(&meta, true).unwrap();
        }
        store
    }

    #[test]
    fn payloads_carry_only_documented_properties_and_no_private_values() {
        let rig = rig_with(
            ON,
            false,
            MockTransport::answering(&[]),
            store_full_of_sentinels(),
        );
        for event in TelemetryEvent::ALL {
            // Unwired events are never emitted; their schema is still checked.
            assert_eq!(rig.shared.emit(event), event.sent_by_this_build());
        }
        while rig.shared.deliver_once() {}
        let bodies = rig.transport.bodies.lock().unwrap().clone();
        let ledger = rig.store.0.lock().unwrap().telemetry_ledger(100).unwrap();
        let mut text = serde_json::to_string(&bodies).unwrap();
        for row in &ledger {
            text.push_str(&row.properties_json);
        }
        for sentinel in SENTINELS {
            assert!(!text.contains(sentinel), "{sentinel} leaked");
        }
        let sent: Vec<_> = TelemetryEvent::ALL
            .into_iter()
            .filter(|e| e.sent_by_this_build())
            .collect();
        let events: Vec<_> = bodies
            .iter()
            .flat_map(|b| b["batch"].as_array().unwrap().clone())
            .collect();
        assert_eq!(events.len(), sent.len());
        for (event, kind) in events.iter().zip(&sent) {
            assert_eq!(event["event"], kind.name());
            let keys: BTreeSet<_> = event
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                BTreeSet::from(["event", "distinct_id", "timestamp", "properties"])
            );
            let properties = event["properties"].as_object().unwrap();
            let mut expected: BTreeSet<_> = COMMON_PROPERTIES.into_iter().collect();
            expected.extend(kind.specific_names());
            expected.insert("$set_once");
            assert_eq!(
                properties
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
                expected
            );
            assert_eq!(properties["surface"], "desktop");
            assert_eq!(properties["os_version"], "15.5");
            // Only enum values: the raw platform of the third session is dropped.
            assert_eq!(
                properties["hosts_detected"],
                serde_json::json!(["claude", "codex"])
            );
            assert_eq!(
                properties["$set_once"],
                serde_json::json!({"desktop_user": true})
            );
        }
        // Every ledger row holds exactly the properties sent, without the ID.
        for row in &ledger {
            let stored: serde_json::Value = serde_json::from_str(&row.properties_json).unwrap();
            assert!(stored.get("$set_once").is_none());
            assert!(stored.get("surface").is_some());
        }
    }

    #[test]
    fn the_install_id_is_saved_once_and_reused() {
        let store = TestStore::new(Store::open_in_memory().unwrap());
        use sender::TelemetryStore;
        let first = store.distinct_id().unwrap();
        assert_eq!(store.distinct_id().unwrap(), first);
    }

    /// One table's rows between its markers: each row's cells, trimmed.
    fn table(document: &str, name: &str) -> Vec<Vec<String>> {
        let start = format!("<!-- telemetry-{name}:start -->");
        let end = format!("<!-- telemetry-{name}:end -->");
        let body = &document
            [document.find(&start).expect(&start) + start.len()..document.find(&end).expect(&end)];
        body.lines()
            .map(str::trim)
            .filter(|line| line.starts_with("| `"))
            .map(|line| {
                line.trim_matches('|')
                    .split('|')
                    .map(|cell| cell.trim().to_owned())
                    .collect()
            })
            .collect()
    }

    /// The backticked words of a cell, in order.
    fn code(cell: &str) -> Vec<String> {
        cell.split('`')
            .skip(1)
            .step_by(2)
            .map(str::to_owned)
            .collect()
    }

    fn telemetry_md() -> String {
        std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../TELEMETRY.md"
        ))
        .unwrap()
    }

    #[test]
    fn telemetry_md_lists_exactly_the_typed_events_and_properties() {
        let document = telemetry_md();
        let events = table(&document, "events");
        assert_eq!(events.len(), TelemetryEvent::ALL.len());
        for (row, event) in events.iter().zip(TelemetryEvent::ALL) {
            assert_eq!(code(&row[0]), [event.name()], "event order and names");
            let documented: Vec<String> = if row[1] == "none" {
                Vec::new()
            } else {
                code(&row[1])
            };
            assert_eq!(documented, event.specific_names(), "{}", event.name());
            let sent = row[2].starts_with("Yes");
            assert!(sent || row[2].starts_with("Not sent yet"), "{}", row[2]);
            assert_eq!(sent, event.sent_by_this_build(), "{}", event.name());
        }
        let common = table(&document, "common");
        let names: Vec<_> = common.iter().map(|row| code(&row[0]).concat()).collect();
        assert_eq!(names, COMMON_PROPERTIES);
        let hosts = common
            .iter()
            .find(|row| row[0] == "`hosts_detected`")
            .unwrap();
        let values: BTreeSet<_> = code(&hosts[1]).into_iter().collect();
        let expected: BTreeSet<_> = DetectedHost::ALL
            .iter()
            .map(|h| {
                serde_json::to_value(h)
                    .unwrap()
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(values, expected);
        // The environment override and its values are documented as shipped.
        for value in consent::ENV_OFF_VALUES {
            assert!(
                document.contains(&format!("`{}={value}`", consent::ENV)),
                "{value}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_event_list_opens_only_the_fixed_published_page() {
        let command = telemetry_doc_command().unwrap();
        assert_eq!(command.get_program(), "/usr/bin/open");
        let args: Vec<_> = command.get_args().collect();
        assert_eq!(args, [TELEMETRY_DOC_URL]);
        assert_eq!(
            TELEMETRY_DOC_URL,
            format!(
                "https://github.com/XTraceAI/xtrace-desktop/blob/v{}/TELEMETRY.md",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn no_analytics_script_runs_in_the_webview() {
        let manifest =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/package.json"))
                .unwrap();
        assert!(!manifest.to_ascii_lowercase().contains("posthog"));
    }
}
