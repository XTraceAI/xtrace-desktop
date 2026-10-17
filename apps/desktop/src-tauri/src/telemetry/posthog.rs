//! The PostHog request: which project and host this build sends to, the
//! batch body, and the one production transport.
//!
//! The project key and host are fixed when the app is built. By default they
//! are XTrace's project token and US host below; `XTRACE_POSTHOG_KEY` and
//! `XTRACE_POSTHOG_HOST`, when set at build time, replace them, and an empty
//! `XTRACE_POSTHOG_KEY` builds an app with no sender at all. Nothing runs in the webview:
//! no PostHog script, autocapture or session replay.
use serde_json::{Map, Value, json};

/// Used when the build names no host.
pub const DEFAULT_HOST: &str = "https://us.i.posthog.com";
/// XTrace's PostHog project token, used when the build names no key. It is a
/// public, write-only token: it can send events to the project, not read any.
pub const DEFAULT_KEY: &str = "phc_Av67luYxroa4WFer6BtN7CwZWtO9LzDKPHmeyij681K";
/// PostHog's batch capture path under the host.
const BATCH_PATH: &str = "/batch/";
/// The longest a single request may take, connection included.
pub const REQUEST_TIMEOUT_SECONDS: u32 = 15;

/// Where this build sends, when it sends at all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PostHogConfig {
    key: String,
    endpoint: String,
}

impl PostHogConfig {
    /// The values compiled into this build.
    pub fn from_build() -> Option<Self> {
        Self::new(
            build_key(option_env!("XTRACE_POSTHOG_KEY")),
            option_env!("XTRACE_POSTHOG_HOST"),
        )
    }

    /// No key, or a key or host of the wrong shape, means no sender. Only an
    /// `https://` host without a path, query or credentials is accepted.
    pub fn new(key: Option<&str>, host: Option<&str>) -> Option<Self> {
        let key = key?.trim();
        let key_valid = !key.is_empty()
            && key.len() <= 128
            && key
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        if !key_valid {
            return None;
        }
        let host = host
            .map(str::trim)
            .filter(|h| !h.is_empty())
            .unwrap_or(DEFAULT_HOST);
        let host = host.strip_suffix('/').unwrap_or(host);
        let authority = host.strip_prefix("https://")?;
        let authority_valid = !authority.is_empty()
            && authority
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b':');
        if !authority_valid {
            return None;
        }
        Some(Self {
            key: key.into(),
            endpoint: format!("https://{authority}{BATCH_PATH}"),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// The JSON body for one batch request.
    pub fn batch_body(&self, events: &[Value]) -> Vec<u8> {
        serde_json::to_vec(&json!({ "api_key": self.key, "batch": events }))
            .expect("a JSON value always serializes")
    }
}

/// The key a build uses: the compiled-in project token unless
/// `XTRACE_POSTHOG_KEY` was set when building. Set but empty, it leaves the
/// build with no key, so no sender.
fn build_key(env: Option<&str>) -> Option<&str> {
    Some(env.unwrap_or(DEFAULT_KEY))
}

/// One PostHog event object: the documented properties plus the anonymous
/// install ID and the person flag set once.
pub fn event_object(
    name: &str,
    distinct_id: &str,
    timestamp_ms: i64,
    properties: &Map<String, Value>,
) -> Value {
    let mut properties = properties.clone();
    properties.insert("$set_once".into(), json!({ "desktop_user": true }));
    let timestamp = jiff::Timestamp::from_millisecond(timestamp_ms)
        .map(|t| t.to_string())
        .unwrap_or_default();
    json!({
        "event": name,
        "distinct_id": distinct_id,
        "timestamp": timestamp,
        "properties": properties,
    })
}

/// The system `curl` on macOS: an HTTPS POST of the body, given on standard
/// input so neither the key nor the events appear in the process list. The
/// user's `.curlrc` is ignored. A cancel stops the request by ending the
/// process. This uses no HTTP library in the app.
#[cfg(target_os = "macos")]
pub struct CurlTransport {
    endpoint: String,
}

#[cfg(target_os = "macos")]
impl CurlTransport {
    pub const PROGRAM: &str = "/usr/bin/curl";

    pub fn new(config: &PostHogConfig) -> Option<Self> {
        std::path::Path::new(Self::PROGRAM).is_file().then(|| Self {
            endpoint: config.endpoint.clone(),
        })
    }

    fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new(Self::PROGRAM);
        command
            .arg("-q")
            .args(["--silent", "--show-error"])
            .args(["--proto", "=https", "--tlsv1.2"])
            .args(["--max-time", &REQUEST_TIMEOUT_SECONDS.to_string()])
            .args(["--output", "/dev/null"])
            .args(["--write-out", "%{http_code}"])
            .args(["--header", "Content-Type: application/json"])
            .args(["--data-binary", "@-"])
            .arg(&self.endpoint)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        command
    }
}

#[cfg(target_os = "macos")]
impl super::sender::Transport for CurlTransport {
    fn send(
        &self,
        body: &[u8],
        cancel: &super::sender::Cancel,
    ) -> Result<(), super::sender::SendError> {
        use super::sender::SendError;
        use std::io::{Read, Write};
        if cancel.is_cancelled() {
            return Err(SendError::Cancelled);
        }
        let mut child = self.command().spawn().map_err(|_| SendError::Failed)?;
        let written = child
            .stdin
            .take()
            .map(|mut stdin| stdin.write_all(body))
            .is_some_and(|result| result.is_ok());
        let status = loop {
            if cancel.is_cancelled() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(SendError::Cancelled);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(std::time::Duration::from_millis(50)),
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(SendError::Failed);
                }
            }
        };
        let mut code = String::new();
        if let Some(mut stdout) = child.stdout.take() {
            let _ = stdout.read_to_string(&mut code);
        }
        if written && status.success() && accepted(&code) {
            Ok(())
        } else {
            Err(SendError::Failed)
        }
    }
}

/// Whether curl's `%{http_code}` output is a 2xx response.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn accepted(code: &str) -> bool {
    let code = code.trim();
    code.len() == 3 && code.starts_with('2') && code.bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_means_no_sender() {
        assert_eq!(PostHogConfig::new(None, None), None);
        assert_eq!(PostHogConfig::new(Some("  "), None), None);
        assert_eq!(PostHogConfig::new(Some("phc key"), None), None);
    }

    #[test]
    fn builds_use_the_project_token_unless_the_key_is_overridden_or_emptied() {
        let default = PostHogConfig::new(build_key(None), None).unwrap();
        assert_eq!(default.key, DEFAULT_KEY);
        assert_eq!(default.endpoint(), "https://us.i.posthog.com/batch/");
        let other = PostHogConfig::new(build_key(Some("phc_other")), None).unwrap();
        assert_eq!(other.key, "phc_other");
        assert_eq!(PostHogConfig::new(build_key(Some("")), None), None);
        assert_eq!(PostHogConfig::new(build_key(Some("  ")), None), None);
    }

    #[test]
    fn the_host_defaults_and_must_be_plain_https() {
        let config = PostHogConfig::new(Some("phc_test"), None).unwrap();
        assert_eq!(config.endpoint(), "https://us.i.posthog.com/batch/");
        let eu = PostHogConfig::new(Some("phc_test"), Some("https://eu.i.posthog.com/")).unwrap();
        assert_eq!(eu.endpoint(), "https://eu.i.posthog.com/batch/");
        for host in [
            "http://us.i.posthog.com",
            "https://user:pw@example.com",
            "https://example.com/path",
            "https://example.com?x=1",
            "ftp://example.com",
        ] {
            assert_eq!(
                PostHogConfig::new(Some("phc_test"), Some(host)),
                None,
                "{host}"
            );
        }
    }

    #[test]
    fn the_body_names_the_project_and_carries_the_events() {
        let config = PostHogConfig::new(Some("phc_test"), None).unwrap();
        let mut properties = Map::new();
        properties.insert("surface".into(), json!("desktop"));
        let event = event_object("app_opened", "id-1", 1_800_000_000_000, &properties);
        let body: Value = serde_json::from_slice(&config.batch_body(&[event])).unwrap();
        assert_eq!(body["api_key"], "phc_test");
        let sent = &body["batch"][0];
        assert_eq!(sent["event"], "app_opened");
        assert_eq!(sent["distinct_id"], "id-1");
        assert_eq!(sent["timestamp"], "2027-01-15T08:00:00Z");
        assert_eq!(sent["properties"]["surface"], "desktop");
        assert_eq!(
            sent["properties"]["$set_once"],
            json!({"desktop_user": true})
        );
    }

    #[test]
    fn only_a_two_hundred_status_is_accepted() {
        for code in ["200", "204", " 200\n"] {
            assert!(accepted(code), "{code:?}");
        }
        for code in ["", "000", "400", "500", "2000", "20x", "301"] {
            assert!(!accepted(code), "{code:?}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn curl_reads_the_body_from_standard_input_and_ignores_curlrc() {
        let config = PostHogConfig::new(Some("phc_secret"), None).unwrap();
        let transport = CurlTransport {
            endpoint: config.endpoint.clone(),
        };
        let command = transport.command();
        assert_eq!(command.get_program(), "/usr/bin/curl");
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_str().unwrap().to_owned())
            .collect();
        assert_eq!(args.first().map(String::as_str), Some("-q"));
        assert!(args.windows(2).any(|w| w == ["--data-binary", "@-"]));
        assert!(args.windows(2).any(|w| w == ["--proto", "=https"]));
        assert_eq!(args.last().map(String::as_str), Some(config.endpoint()));
        assert!(!args.iter().any(|a| a.contains("phc_secret")));
    }
}
