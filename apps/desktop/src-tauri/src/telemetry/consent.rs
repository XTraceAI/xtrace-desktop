//! The one rule for whether product analytics may leave this Mac,
//! and the state every screen shows, decided here once.
//!
//! Telemetry is on by default, but nothing may be sent until the disclosure
//! was actually shown, and `XTRACE_TELEMETRY=0` always wins over the saved
//! preference. The sender checks [`can_send`] again before every request.
use serde::Serialize;
use ts_rs::TS;

/// The environment variable that turns telemetry off whatever was saved.
pub const ENV: &str = "XTRACE_TELEMETRY";

/// Values of [`ENV`] that turn telemetry off, compared trimmed and ignoring
/// case. Any other value, or none, leaves the saved preference in charge.
pub const ENV_OFF_VALUES: [&str; 4] = ["0", "false", "off", "no"];

/// The single gate: true only when the disclosure was shown, the saved
/// preference is on and the environment does not turn telemetry off.
pub fn can_send(disclosure_shown: bool, preference_on: bool, env_off: bool) -> bool {
    disclosure_shown && preference_on && !env_off
}

/// Whether this value of [`ENV`] turns telemetry off.
pub fn env_off(value: Option<&std::ffi::OsStr>) -> bool {
    value
        .and_then(|value| value.to_str())
        .map(str::trim)
        .is_some_and(|value| {
            ENV_OFF_VALUES
                .iter()
                .any(|off| value.eq_ignore_ascii_case(off))
        })
}

/// The two saved consent facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Consent {
    pub preference_on: bool,
    pub disclosure_shown: bool,
}

impl Consent {
    /// What a fresh install reads, and what an unreadable store is treated
    /// as: on by default, never shown, so nothing can be sent.
    pub const FRESH: Self = Self {
        preference_on: true,
        disclosure_shown: false,
    };
}

impl From<xt_store::telemetry::TelemetrySettings> for Consent {
    fn from(saved: xt_store::telemetry::TelemetrySettings) -> Self {
        Self {
            preference_on: saved.enabled,
            disclosure_shown: saved.disclosure_shown,
        }
    }
}

/// The running app's copy of the consent facts the sender checks.
///
/// Saved facts are applied here only by the write that saved them, while it
/// still holds the store, so this copy follows the order the store saved
/// changes in; a read never changes it. Turning telemetry off is applied here
/// before it is saved and holds until the user turns telemetry on again, so a
/// failed save, or a write that read the store before the user turned it off,
/// cannot reopen it for this launch.
#[derive(Debug)]
pub struct Gate {
    facts: std::sync::Mutex<GateFacts>,
    env_off: bool,
}

#[derive(Debug)]
struct GateFacts {
    /// The facts the store last reported saving.
    saved: Consent,
    /// The user turned telemetry off in this launch and has not turned it on.
    held_off: bool,
}

impl GateFacts {
    /// What this launch uses: the saved facts, off while a Turn off holds.
    fn effective(&self) -> Consent {
        Consent {
            preference_on: self.saved.preference_on && !self.held_off,
            ..self.saved
        }
    }
    /// Off for this launch only: a Turn off holds, but the store still says on.
    fn off_unsaved(&self) -> bool {
        self.held_off && self.saved.preference_on
    }
}

impl Gate {
    pub fn new(consent: Consent, env_off: bool) -> Self {
        Self {
            facts: std::sync::Mutex::new(GateFacts {
                saved: consent,
                held_off: false,
            }),
            env_off,
        }
    }
    fn facts(&self) -> std::sync::MutexGuard<'_, GateFacts> {
        self.facts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
    /// The facts this launch uses.
    pub fn consent(&self) -> Consent {
        self.facts().effective()
    }
    /// The facts this launch uses, and whether telemetry is off only until
    /// the app restarts (see [`TelemetryStatus::OffUntilRestart`]), read
    /// together.
    pub fn snapshot(&self) -> (Consent, bool) {
        let facts = self.facts();
        (facts.effective(), facts.off_unsaved())
    }
    /// Applies facts a write saved; an in-launch Turn off still holds.
    pub fn set(&self, saved: Consent) {
        self.facts().saved = saved;
    }
    /// The user turned telemetry off: off at once, until [`Self::turn_on`].
    pub fn hold_off(&self) {
        self.facts().held_off = true;
    }
    /// Applies the facts saved by the user turning telemetry on, which ends
    /// an in-launch Turn off once the saved preference is on.
    pub fn turn_on(&self, saved: Consent) {
        let mut facts = self.facts();
        facts.held_off = !saved.preference_on;
        facts.saved = saved;
    }
    pub fn env_off(&self) -> bool {
        self.env_off
    }
    /// [`can_send`] for the current facts.
    pub fn allows(&self) -> bool {
        let consent = self.consent();
        can_send(
            consent.disclosure_shown,
            consent.preference_on,
            self.env_off,
        )
    }
}

/// Why telemetry is or is not being sent, in the order the reasons win.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TelemetryStatus {
    /// `XTRACE_TELEMETRY` turns it off; the saved preference cannot change it.
    EnvironmentOff,
    /// Turned off in this launch, but saving that failed: nothing is sent
    /// now, and the saved preference, still on, applies after a restart.
    OffUntilRestart,
    /// This copy of the app never sends anything: it is a test copy (not the
    /// installed app's identifier), it opened disposable test data, or it was
    /// built without an analytics project key.
    NotConfigured,
    /// The saved preference is off.
    TurnedOff,
    /// On, but the disclosure has not been shown yet, so nothing is sent.
    AwaitingDisclosure,
    /// Events are sent.
    Sending,
}

/// Everything a screen needs about telemetry. The renderer shows these
/// fields and never works any of them out again.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, TS)]
pub struct TelemetryState {
    /// The preference this launch uses: the saved one, except off after the
    /// user turned telemetry off in this launch even if saving that failed
    /// ([`TelemetryStatus::OffUntilRestart`] says when the store still says on).
    pub preference_on: bool,
    /// What an on/off switch shows: [`Self::preference_on`], shown off while
    /// the environment override turns telemetry off.
    pub enabled: bool,
    pub disclosure_shown: bool,
    /// `XTRACE_TELEMETRY` turns telemetry off for this launch.
    pub env_off: bool,
    /// This launch has a sender: the build has a project key, the app runs
    /// under the installed app's identifier and the data is not a disposable
    /// fixture.
    pub configured: bool,
    /// [`can_send`] for the saved facts and the environment.
    pub can_send: bool,
    /// Whether the preference may be changed now (not under the override).
    pub can_change: bool,
    /// Whether the disclosure must be shown: something could be sent once it
    /// is, and it has not been shown yet.
    pub show_disclosure: bool,
    pub status: TelemetryStatus,
}

impl TelemetryState {
    /// `consent` is what this launch uses; `off_unsaved` is true when it is
    /// off only because a Turn off in this launch could not be saved.
    pub fn decide(consent: Consent, env_off: bool, configured: bool, off_unsaved: bool) -> Self {
        let consent = Consent {
            preference_on: consent.preference_on && !off_unsaved,
            ..consent
        };
        let can_send = can_send(consent.disclosure_shown, consent.preference_on, env_off);
        let status = if env_off {
            TelemetryStatus::EnvironmentOff
        } else if off_unsaved {
            TelemetryStatus::OffUntilRestart
        } else if !configured {
            TelemetryStatus::NotConfigured
        } else if !consent.preference_on {
            TelemetryStatus::TurnedOff
        } else if !consent.disclosure_shown {
            TelemetryStatus::AwaitingDisclosure
        } else {
            TelemetryStatus::Sending
        };
        Self {
            preference_on: consent.preference_on,
            enabled: consent.preference_on && !env_off,
            disclosure_shown: consent.disclosure_shown,
            env_off,
            configured,
            can_send,
            can_change: !env_off,
            show_disclosure: status == TelemetryStatus::AwaitingDisclosure,
            status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    const BOOLS: [bool; 2] = [false, true];

    #[test]
    fn gate_truth_table() {
        for disclosure_shown in BOOLS {
            for preference_on in BOOLS {
                for env_off in BOOLS {
                    assert_eq!(
                        can_send(disclosure_shown, preference_on, env_off),
                        disclosure_shown && preference_on && !env_off,
                        "{disclosure_shown} {preference_on} {env_off}"
                    );
                    for (configured, off_unsaved) in
                        [(false, false), (true, false), (false, true), (true, true)]
                    {
                        let state = TelemetryState::decide(
                            Consent {
                                preference_on,
                                disclosure_shown,
                            },
                            env_off,
                            configured,
                            off_unsaved,
                        );
                        let preference_on = preference_on && !off_unsaved;
                        assert_eq!(
                            state.status == TelemetryStatus::OffUntilRestart,
                            off_unsaved && !env_off
                        );
                        // The DTO carries the gate unchanged.
                        assert_eq!(
                            state.can_send,
                            can_send(disclosure_shown, preference_on, env_off)
                        );
                        assert_eq!(state.can_change, !env_off);
                        assert_eq!(state.enabled, preference_on && !env_off);
                        // `Sending` is shown exactly when a configured sender may send.
                        assert_eq!(
                            state.status == TelemetryStatus::Sending,
                            state.can_send && configured
                        );
                        assert_eq!(
                            state.show_disclosure,
                            configured && preference_on && !disclosure_shown && !env_off
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_fresh_install_is_on_but_cannot_send() {
        let state = TelemetryState::decide(Consent::FRESH, false, true, false);
        assert!(state.preference_on);
        assert!(!state.can_send);
        assert!(state.show_disclosure);
        assert_eq!(state.status, TelemetryStatus::AwaitingDisclosure);
    }

    #[test]
    fn a_stored_on_cannot_override_the_environment() {
        let shown_and_on = Consent {
            preference_on: true,
            disclosure_shown: true,
        };
        let state = TelemetryState::decide(shown_and_on, true, true, false);
        assert!(!state.can_send);
        assert!(!state.can_change);
        assert_eq!(state.status, TelemetryStatus::EnvironmentOff);
    }

    #[test]
    fn the_environment_values_that_turn_it_off() {
        for value in ["0", " 0 ", "false", "FALSE", "off", "Off", "no"] {
            assert!(env_off(Some(OsStr::new(value))), "{value:?}");
        }
        for value in ["", "1", "true", "on", "yes", "00", "disabled"] {
            assert!(!env_off(Some(OsStr::new(value))), "{value:?}");
        }
        assert!(!env_off(None));
    }
}
