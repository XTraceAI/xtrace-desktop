//! The typed allowlist of product-analytics events and the only
//! properties each may carry. Every value is a closed enum, the build's own
//! version, or the operating system version: no free text from the user's
//! data can reach a payload. `TELEMETRY.md` documents exactly this list; a
//! test keeps the two the same.
use serde::Serialize;
use serde_json::{Map, Value, json};

/// A coding agent whose history the local index holds. Only these values are
/// ever sent; a session from any other host is not reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetectedHost {
    Claude,
    Codex,
    Cursor,
}

impl DetectedHost {
    pub const ALL: [Self; 3] = [Self::Claude, Self::Codex, Self::Cursor];
    pub fn from_store(host: xt_store::Host) -> Option<Self> {
        match host {
            xt_store::Host::Claude => Some(Self::Claude),
            xt_store::Host::Codex => Some(Self::Codex),
            xt_store::Host::Cursor => Some(Self::Cursor),
            xt_store::Host::Other => None,
        }
    }
    fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
        }
    }
}

/// A rule's enforcement class.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleClass {
    Hook,
    Linter,
    Test,
    Judge,
}

/// A rule's delivery mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleMode {
    Advise,
    Gate,
}

/// Every event the app may send. Variants whose feature does not exist in
/// this build are defined so the schema is fixed, and are never emitted
/// ([`TelemetryEvent::sent_by_this_build`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TelemetryEvent {
    AppOpened,
    DailyActive,
    HourlyActive,
    PluginDetected { host: DetectedHost },
    PluginInstalled { host: DetectedHost },
    RuleActivated { class: RuleClass, mode: RuleMode },
    LeaderboardOptedIn,
    CloudConnected,
    UpdateInstalled,
}

/// Properties every event carries, in the order they are documented.
pub const COMMON_PROPERTIES: [&str; 4] = ["surface", "app_version", "os_version", "hosts_detected"];

/// The value of `surface` on every desktop event.
pub const SURFACE: &str = "desktop";

impl TelemetryEvent {
    /// One example of each event, in documentation order.
    pub const ALL: [Self; 9] = [
        Self::AppOpened,
        Self::DailyActive,
        Self::HourlyActive,
        Self::PluginDetected {
            host: DetectedHost::Claude,
        },
        Self::PluginInstalled {
            host: DetectedHost::Claude,
        },
        Self::RuleActivated {
            class: RuleClass::Hook,
            mode: RuleMode::Advise,
        },
        Self::LeaderboardOptedIn,
        Self::CloudConnected,
        Self::UpdateInstalled,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::AppOpened => "app_opened",
            Self::DailyActive => "daily_active",
            Self::HourlyActive => "hourly_active",
            Self::PluginDetected { .. } => "plugin_detected",
            Self::PluginInstalled { .. } => "plugin_installed",
            Self::RuleActivated { .. } => "rule_activated",
            Self::LeaderboardOptedIn => "leaderboard_opted_in",
            Self::CloudConnected => "cloud_connected",
            Self::UpdateInstalled => "update_installed",
        }
    }

    /// Whether this build has a real trigger for the event. The others are
    /// documented as not sent yet.
    pub fn sent_by_this_build(self) -> bool {
        match self {
            Self::AppOpened | Self::DailyActive | Self::HourlyActive => true,
            Self::PluginDetected { .. }
            | Self::PluginInstalled { .. }
            | Self::RuleActivated { .. }
            | Self::LeaderboardOptedIn
            | Self::CloudConnected
            | Self::UpdateInstalled => false,
        }
    }

    /// The event's own properties, after the common ones.
    fn specific(self) -> Vec<(&'static str, Value)> {
        match self {
            Self::PluginDetected { host } | Self::PluginInstalled { host } => {
                vec![("host", json!(host.as_str()))]
            }
            Self::RuleActivated { class, mode } => vec![
                (
                    "class",
                    json!(match class {
                        RuleClass::Hook => "hook",
                        RuleClass::Linter => "linter",
                        RuleClass::Test => "test",
                        RuleClass::Judge => "judge",
                    }),
                ),
                (
                    "mode",
                    json!(match mode {
                        RuleMode::Advise => "advise",
                        RuleMode::Gate => "gate",
                    }),
                ),
            ],
            Self::AppOpened
            | Self::DailyActive
            | Self::HourlyActive
            | Self::LeaderboardOptedIn
            | Self::CloudConnected
            | Self::UpdateInstalled => Vec::new(),
        }
    }

    /// The names of the event's own properties.
    pub fn specific_names(self) -> Vec<&'static str> {
        self.specific().into_iter().map(|(name, _)| name).collect()
    }

    /// Exactly the documented properties this event is sent with.
    pub fn properties(self, common: &CommonProperties) -> Map<String, Value> {
        let mut hosts = common.hosts_detected.clone();
        hosts.sort();
        hosts.dedup();
        let mut properties = Map::new();
        properties.insert("surface".into(), json!(SURFACE));
        properties.insert("app_version".into(), json!(common.app_version.as_str()));
        properties.insert("os_version".into(), json!(common.os_version.as_str()));
        properties.insert(
            "hosts_detected".into(),
            Value::Array(hosts.into_iter().map(|h| json!(h.as_str())).collect()),
        );
        for (name, value) in self.specific() {
            properties.insert(name.into(), value);
        }
        properties
    }
}

/// The values every event carries, read when the event is made.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommonProperties {
    pub app_version: AppVersion,
    pub os_version: OsVersion,
    pub hosts_detected: Vec<DetectedHost>,
}

/// A version made only of digits and dots (at most 3 dots, 32 characters),
/// or `unknown`. Anything else read from the system is never sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OsVersion(String);

impl OsVersion {
    pub fn parse(text: &str) -> Self {
        let text = text.trim();
        let valid = !text.is_empty()
            && text.len() <= 32
            && text.split('.').count() <= 4
            && text
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
        Self(if valid { text.into() } else { "unknown".into() })
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The macOS product version from the system's own version file;
    /// `unknown` elsewhere or when it cannot be read.
    pub fn current() -> Self {
        #[cfg(target_os = "macos")]
        {
            const FILE: &str = "/System/Library/CoreServices/SystemVersion.plist";
            if let Ok(plist) = std::fs::read_to_string(FILE)
                && let Some(version) = plist_string(&plist, "ProductVersion")
            {
                return Self::parse(version);
            }
        }
        Self::parse("")
    }
}

/// The app's own version, digits and dots with an optional pre-release tag
/// of letters, digits, dots and hyphens; `unknown` otherwise.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppVersion(String);

impl AppVersion {
    pub fn parse(text: &str) -> Self {
        let valid = !text.is_empty()
            && text.len() <= 32
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-');
        Self(if valid { text.into() } else { "unknown".into() })
    }
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The `<string>` that follows `<key>name</key>` in an XML property list.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn plist_string<'a>(plist: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("<key>{name}</key>");
    let rest = plist[plist.find(&key)? + key.len()..].trim_start();
    let rest = rest.strip_prefix("<string>")?;
    Some(&rest[..rest.find("</string>")?])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_event_has_a_distinct_name_and_only_enum_properties() {
        let names: std::collections::BTreeSet<_> =
            TelemetryEvent::ALL.iter().map(|e| e.name()).collect();
        assert_eq!(names.len(), TelemetryEvent::ALL.len());
        let common = CommonProperties {
            app_version: AppVersion::parse("1.2.3"),
            os_version: OsVersion::parse("15.5"),
            hosts_detected: vec![
                DetectedHost::Codex,
                DetectedHost::Claude,
                DetectedHost::Codex,
            ],
        };
        for event in TelemetryEvent::ALL {
            let properties = event.properties(&common);
            let mut keys: Vec<_> = properties.keys().map(String::as_str).collect();
            keys.sort();
            let mut expected: Vec<_> = COMMON_PROPERTIES.to_vec();
            expected.extend(event.specific_names());
            expected.sort();
            assert_eq!(keys, expected, "{}", event.name());
            assert_eq!(properties["surface"], "desktop");
            assert_eq!(properties["hosts_detected"], json!(["claude", "codex"]));
        }
    }

    #[test]
    fn versions_outside_the_allowed_shape_are_unknown() {
        assert_eq!(OsVersion::parse("15.5.1").as_str(), "15.5.1");
        for bad in ["", "15..1", "/Users/x", "15.5 (24F74)", "1.2.3.4.5", "a"] {
            assert_eq!(OsVersion::parse(bad).as_str(), "unknown", "{bad}");
        }
        assert_eq!(AppVersion::parse("0.1.0-beta.14").as_str(), "0.1.0-beta.14");
        assert_eq!(AppVersion::parse("0.1/x").as_str(), "unknown");
        assert_eq!(AppVersion::current().as_str(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn the_product_version_is_read_from_a_property_list() {
        let plist = "<dict>\n\t<key>ProductName</key>\n\t<string>macOS</string>\n\t<key>ProductVersion</key>\n\t<string>15.5</string>\n</dict>";
        assert_eq!(plist_string(plist, "ProductVersion"), Some("15.5"));
        assert_eq!(plist_string(plist, "Missing"), None);
    }

    #[test]
    fn only_known_hosts_are_reported() {
        assert_eq!(DetectedHost::from_store(xt_store::Host::Other), None);
        for host in DetectedHost::ALL {
            let store = match host {
                DetectedHost::Claude => xt_store::Host::Claude,
                DetectedHost::Codex => xt_store::Host::Codex,
                DetectedHost::Cursor => xt_store::Host::Cursor,
            };
            assert_eq!(DetectedHost::from_store(store), Some(host));
        }
    }
}
