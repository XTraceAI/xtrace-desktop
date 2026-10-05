//! A one-shot child reads Claude Code's exact Keychain item, then account usage.
//! Used only as the fallback of an explicit Refresh usage click when reading
//! through the user's own Claude Code fails; automatic reads never use it.
use crate::account_usage::{available, failed, parse_claude, unavailable};
use crate::account_usage_dto::{AccountProviderUsage, AccountUsageIssue};
use crate::account_usage_process::{ProcessFailure, run_to_exit};
use serde_json::Value;
use std::{io::Write, process::Command, time::Duration};

pub(super) const HELPER_FLAG: &str = "--xtrace-private-claude-usage-once";

/// Parent-side only. The child owns both the raw credential and the curl
/// header; the parent receives only a sanitized provider DTO over stdout.
pub(super) fn read_via_helper() -> AccountProviderUsage {
    if helper_fixture_guard() {
        return unavailable(AccountUsageIssue::SourceUnavailable);
    }
    let Ok(executable) = std::env::current_exe() else {
        return failed(AccountUsageIssue::SourceUnavailable);
    };
    let mut command = Command::new(executable);
    command.arg(HELPER_FLAG);
    let output = match run_to_exit(&mut command, b"", Duration::from_secs(12), 16_384, true) {
        Ok(output) => output,
        Err(ProcessFailure::Timeout) => return failed(AccountUsageIssue::Timeout),
        Err(ProcessFailure::OutputTooLarge) => return failed(AccountUsageIssue::InvalidResponse),
        Err(_) => return failed(AccountUsageIssue::SourceUnavailable),
    };
    let Ok(report) = serde_json::from_slice::<AccountProviderUsage>(&output) else {
        return failed(AccountUsageIssue::InvalidResponse);
    };
    if !super::account_usage::valid_report(&report) {
        return failed(AccountUsageIssue::InvalidResponse);
    }
    report
}

/// Called by the executable before any Tauri, webview, database, or UI setup.
/// This mode has no renderer command and accepts no URL or credential input.
pub(super) fn run_one_shot() -> i32 {
    let report = one_shot_report(helper_fixture_guard(), read);
    let Ok(output) = serde_json::to_vec(&report) else {
        return 1;
    };
    let mut stdout = std::io::stdout().lock();
    if stdout.write_all(&output).is_err() {
        return 1;
    }
    0
}

fn helper_fixture_guard() -> bool {
    cfg!(feature = "fixtures") || std::env::var_os("XTRACE_FIXTURE").is_some()
}

fn one_shot_report(
    fixture: bool,
    provider_read: impl FnOnce() -> AccountProviderUsage,
) -> AccountProviderUsage {
    if fixture {
        unavailable(AccountUsageIssue::SourceUnavailable)
    } else {
        provider_read()
    }
}

fn read() -> AccountProviderUsage {
    let token = match access_token() {
        Ok(token) => token,
        Err(AccountUsageIssue::CredentialAccessRequired) => {
            return unavailable(AccountUsageIssue::CredentialAccessRequired);
        }
        Err(issue) => return unavailable(issue),
    };
    // curl reads all headers from stdin. The fixed URL is the only remote
    // destination; redirects, proxies from environment, and retries are off.
    let mut command = Command::new("/usr/bin/curl");
    command.args([
        "--disable",
        "--silent",
        "--show-error",
        "--globoff",
        "--request",
        "GET",
        "--proto",
        "=https",
        "--max-redirs",
        "0",
        "--noproxy",
        "*",
        "--connect-timeout",
        "3",
        "--max-time",
        "8",
        "--max-filesize",
        "65536",
        "--header",
        "@-",
        "--write-out",
        "\n%{http_code}",
        "https://api.anthropic.com/api/oauth/usage",
    ]);
    command.env_remove("http_proxy");
    command.env_remove("https_proxy");
    command.env_remove("HTTP_PROXY");
    command.env_remove("HTTPS_PROXY");
    command.env_remove("ALL_PROXY");
    let headers = format!(
        "Authorization: Bearer {token}\nanthropic-beta: oauth-2025-04-20\nAccept: application/json\nUser-Agent: claude-code/2.1.0\n"
    );
    let output = match run_to_exit(
        &mut command,
        headers.as_bytes(),
        Duration::from_secs(8),
        65_550,
        false,
    ) {
        Ok(output) => output,
        Err(ProcessFailure::Timeout) => return failed(AccountUsageIssue::Timeout),
        Err(ProcessFailure::OutputTooLarge) => return failed(AccountUsageIssue::InvalidResponse),
        Err(_) => return failed(AccountUsageIssue::Network),
    };
    let Some(split) = output.iter().rposition(|byte| *byte == b'\n') else {
        return failed(AccountUsageIssue::Network);
    };
    let (body, status) = (&output[..split], &output[split + 1..]);
    match status {
        b"401" => return unavailable(AccountUsageIssue::Unauthorized),
        b"429" => return failed(AccountUsageIssue::RateLimited),
        b"200" => {}
        _ => return failed(AccountUsageIssue::Network),
    }
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return failed(AccountUsageIssue::InvalidResponse);
    };
    match parse_claude(&value) {
        Ok(windows) => available(windows),
        Err(AccountUsageIssue::SourceUnavailable) => {
            unavailable(AccountUsageIssue::SourceUnavailable)
        }
        Err(issue) => failed(issue),
    }
}

#[cfg(target_os = "macos")]
fn access_token() -> Result<String, AccountUsageIssue> {
    // Only this private one-shot helper receives the raw credential. Apple's
    // security tool matches the Claude Code Keychain item on this host; it may
    // still need access on another host, so this attempt stays user-triggered.
    let mut command = Command::new("/usr/bin/security");
    command.args([
        "find-generic-password",
        "-s",
        "Claude Code-credentials",
        "-w",
    ]);
    // Inherit the one-shot helper's process group so its outer deadline can
    // stop this child too if the helper stalls during its own cleanup.
    let data = match run_to_exit(&mut command, b"", Duration::from_secs(2), 65_536, false) {
        Ok(data) => data,
        Err(ProcessFailure::Timeout) => return Err(AccountUsageIssue::CredentialAccessRequired),
        Err(_) => return Err(AccountUsageIssue::CredentialUnavailable),
    };
    parse_access_token(&data)
}

fn parse_access_token(data: &[u8]) -> Result<String, AccountUsageIssue> {
    let value: Value =
        serde_json::from_slice(data).map_err(|_| AccountUsageIssue::CredentialUnavailable)?;
    let token = value
        .get("claudeAiOauth")
        .and_then(|entry| entry.get("accessToken"))
        .and_then(Value::as_str)
        .ok_or(AccountUsageIssue::CredentialUnavailable)?;
    // Header injection is impossible even if a corrupted credential item was
    // read. OAuth bearer tokens are printable, single-line ASCII.
    if token.is_empty()
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_graphic() && byte != b'"' && byte != b'\\')
    {
        return Err(AccountUsageIssue::CredentialUnavailable);
    }
    Ok(token.to_owned())
}

#[cfg(not(target_os = "macos"))]
fn access_token() -> Result<String, AccountUsageIssue> {
    Err(AccountUsageIssue::CredentialUnavailable)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_helper_returns_before_credential_or_network_reader() {
        let result = one_shot_report(true, || panic!("live reader must not run"));
        assert_eq!(
            result.state,
            crate::account_usage_dto::AccountUsageState::Unavailable
        );
        assert!(result.windows.is_empty());
    }

    #[test]
    fn credential_parser_accepts_only_a_single_safe_bearer_value() {
        let token =
            parse_access_token(br#"{"claudeAiOauth":{"accessToken":"synthetic-token"}}"#).unwrap();
        assert_eq!(token, "synthetic-token");
        assert!(parse_access_token(br#"{"claudeAiOauth":{"accessToken":"bad\nheader"}}"#).is_err());
        assert!(parse_access_token(br#"{"mcpOAuth":{}}"#).is_err());
    }
}
