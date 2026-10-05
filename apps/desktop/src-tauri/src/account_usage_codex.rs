//! The installed Codex CLI's read-only app-server account limits call.
use crate::account_usage::{available, failed, parse_codex, unavailable};
use crate::account_usage_dto::{AccountProviderUsage, AccountUsageIssue};
use serde_json::Value;
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::{Duration, Instant},
};

pub(super) fn read() -> AccountProviderUsage {
    let Some(executable) = resolve_codex() else {
        return unavailable(AccountUsageIssue::SourceUnavailable);
    };
    let mut command = Command::new(executable);
    command.args(["app-server", "--stdio"]);
    let value = match exchange(&mut command, Duration::from_secs(8)) {
        Ok(value) => value,
        Err(AccountUsageIssue::Timeout) => return failed(AccountUsageIssue::Timeout),
        Err(AccountUsageIssue::InvalidResponse) => {
            return failed(AccountUsageIssue::InvalidResponse);
        }
        Err(issue) => return unavailable(issue),
    };
    match parse_codex(&value) {
        Ok(windows) => available(windows),
        Err(AccountUsageIssue::SourceUnavailable) => {
            unavailable(AccountUsageIssue::SourceUnavailable)
        }
        Err(issue) => failed(issue),
    }
}

const INITIALIZE: &str = "{\"id\":1,\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"name\":\"xtrace\",\"version\":\"0.1.0\"}}}\n";
const RATE_LIMITS: &str = concat!(
    "{\"method\":\"initialized\",\"params\":{}}\n",
    "{\"id\":2,\"method\":\"account/rateLimits/read\",\"params\":{\"excludeResetCreditDetails\":true}}\n",
);
const MAX_OUTPUT: usize = 131_072;

enum Message {
    Json(Value),
    OutputTooLarge,
    End,
}

/// A private, read-only app-server exchange. Stdin remains open until id 2:
/// the installed CLI may drop queued work if it sees EOF before replying.
fn exchange(command: &mut Command, timeout: Duration) -> Result<Value, AccountUsageIssue> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| AccountUsageIssue::SourceUnavailable)?;
    let Some(stdout) = child.stdout.take() else {
        kill_group_and_reap(&mut child);
        return Err(AccountUsageIssue::SourceUnavailable);
    };
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || read_messages(stdout, tx));
    let deadline = Instant::now() + timeout;
    let outcome = (|| {
        let mut stdin = child
            .stdin
            .take()
            .ok_or(AccountUsageIssue::SourceUnavailable)?;
        stdin
            .write_all(INITIALIZE.as_bytes())
            .map_err(|_| AccountUsageIssue::SourceUnavailable)?;
        let initialized = wait_for_id(&rx, 1, deadline)?;
        if initialized.get("error").is_some() {
            return Err(AccountUsageIssue::SourceUnavailable);
        }
        stdin
            .write_all(RATE_LIMITS.as_bytes())
            .map_err(|_| AccountUsageIssue::SourceUnavailable)?;
        let reply = wait_for_id(&rx, 2, deadline)?;
        if reply.get("error").is_some() {
            return Err(AccountUsageIssue::SourceUnavailable);
        }
        Ok(reply)
    })();
    // The protocol exchange is complete or failed. Never leave a CLI process,
    // its descendants, or a reader thread attached to XTrace.
    kill_group_and_reap(&mut child);
    let _ = reader.join();
    outcome
}

fn wait_for_id(
    rx: &Receiver<Message>,
    id: i64,
    deadline: Instant,
) -> Result<Value, AccountUsageIssue> {
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .ok_or(AccountUsageIssue::Timeout)?;
        let message = rx.recv_timeout(left).map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => AccountUsageIssue::Timeout,
            mpsc::RecvTimeoutError::Disconnected => AccountUsageIssue::SourceUnavailable,
        })?;
        match message {
            Message::Json(value) if value.get("id").and_then(Value::as_i64) == Some(id) => {
                return Ok(value);
            }
            Message::Json(_) => {}
            Message::OutputTooLarge => return Err(AccountUsageIssue::InvalidResponse),
            Message::End => return Err(AccountUsageIssue::SourceUnavailable),
        }
    }
}

fn read_messages(mut stdout: impl Read, tx: mpsc::Sender<Message>) {
    let mut total = 0usize;
    let mut line = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let count = match stdout.read(&mut chunk) {
            Ok(0) | Err(_) => break,
            Ok(count) => count,
        };
        total = total.saturating_add(count);
        if total > MAX_OUTPUT {
            let _ = tx.send(Message::OutputTooLarge);
            return;
        }
        for byte in &chunk[..count] {
            if *byte == b'\n' {
                if let Ok(value) = serde_json::from_slice::<Value>(&line) {
                    let _ = tx.send(Message::Json(value));
                }
                line.clear();
            } else {
                line.push(*byte);
            }
        }
    }
    let _ = tx.send(Message::End);
}

fn kill_group_and_reap(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        if let Ok(pid) = i32::try_from(child.id()) {
            let _ = kill(-pid, 9);
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn resolve_codex() -> Option<PathBuf> {
    // Prefer the CLI bundled with the installed ChatGPT app: this is the
    // account source the desktop app can read even with Finder's short PATH.
    let mut candidates = vec![PathBuf::from(
        "/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex",
    )];
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&path)
                .filter(|directory| directory.is_absolute())
                .map(|directory| directory.join("codex")),
        );
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".local/bin/codex"));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/codex"),
        PathBuf::from("/usr/local/bin/codex"),
    ]);
    candidates.into_iter().find(|path| is_executable(path))
}

#[cfg(unix)]
pub(super) fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.is_absolute()
        && path
            .metadata()
            .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
}

#[cfg(not(unix))]
pub(super) fn is_executable(_: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_initialize_then_async_rate_limits_reply_without_closing_stdin() {
        let fake = r#"
import json, select, sys, time
assert json.loads(sys.stdin.readline())['method'] == 'initialize'
print(json.dumps({'id': 1, 'result': {}}), flush=True)
assert json.loads(sys.stdin.readline())['method'] == 'initialized'
assert json.loads(sys.stdin.readline())['method'] == 'account/rateLimits/read'
time.sleep(0.08)
# The prior adapter closed stdin immediately after sending all three lines.
# This fake server then discards the queued account read, like the live CLI.
if select.select([sys.stdin], [], [], 0)[0] and sys.stdin.read(1) == '':
    sys.exit(0)
print(json.dumps({'id': 2, 'result': {'rateLimits': {'primary': {'usedPercent': 29, 'windowDurationMins': 10080, 'resetsAt': 1791050717}, 'secondary': None}}}), flush=True)
"#;
        let mut command = Command::new("python3");
        command.args(["-u", "-c", fake]);
        let reply = exchange(&mut command, Duration::from_secs(2)).unwrap();
        let windows = parse_codex(&reply).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].used_percent, 29.0);
        assert_eq!(windows[0].window, "Week");
    }

    #[cfg(unix)]
    #[test]
    fn timeout_kills_and_reaps_fake_app_server() {
        let pid_file = tempfile::NamedTempFile::new().unwrap();
        let fake = r#"
import json, os, sys, time
with open(sys.argv[1], 'w') as out:
    out.write(str(os.getpid()))
assert json.loads(sys.stdin.readline())['method'] == 'initialize'
print(json.dumps({'id': 1, 'result': {}}), flush=True)
sys.stdin.readline()
sys.stdin.readline()
time.sleep(30)
"#;
        let mut command = Command::new("python3");
        command.args(["-u", "-c", fake]).arg(pid_file.path());
        let started = Instant::now();
        assert!(matches!(
            exchange(&mut command, Duration::from_millis(250)),
            Err(AccountUsageIssue::Timeout)
        ));
        assert!(started.elapsed() < Duration::from_secs(2));
        let pid: i32 = std::fs::read_to_string(pid_file.path())
            .unwrap()
            .parse()
            .unwrap();
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        assert_eq!(
            unsafe { kill(pid, 0) },
            -1,
            "timed-out app-server process remained alive"
        );
    }
}
