//! Reads Claude usage by running the user's own Claude Code CLI in a hidden
//! pseudo-terminal, in a dedicated empty folder under the app's data folder,
//! and typing `/usage`. `/usage` sends no message and uses no quota. XTrace
//! never reads a credential on this path; Claude Code uses its own login.
//!
//! Claude Code saves what `/usage` fetched as `cachedUsageUtilization` in its
//! own config file, which is the primary result. The rendered panel is the
//! fallback. Every run is bounded: one overall deadline, a capped output
//! volume, and the child's whole process group killed if it does not quit.
use crate::account_usage::{available, failed, unavailable};
use crate::account_usage_claude_local::{
    CachedUsage, ClaudeUsagePaths, PROBE_DIR_NAME, TrustAction, parse_usage_screen,
    read_cached_usage, trust_action,
};
use crate::account_usage_claude_screen::Screen;
use crate::account_usage_dto::{
    AccountProviderUsage, AccountUsageIssue, AccountUsageScope, AccountUsageWindow,
};
use jiff::Timestamp;
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const ROWS: u16 = 40;
const COLS: u16 = 120;
/// Overall deadline for one probe, including quitting Claude Code.
const HARD_DEADLINE: Duration = Duration::from_secs(25);
/// Time to interact before the probe starts quitting.
const INTERACT_DEADLINE: Duration = Duration::from_secs(20);
/// Output beyond this is not a usage panel.
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const SETTINGS: &str = r#"{"remoteControlAtStartup":false,"disableAllHooks":true}"#;

pub(super) struct Panel {
    pub(super) text: String,
    /// When the screen showed `text`.
    pub(super) at: Timestamp,
}

pub(super) struct ProbeOutcome {
    /// Wall-clock milliseconds just before Claude Code started.
    pub(super) started_ms: i64,
    /// The screen as it looked when the usage panel was complete.
    pub(super) panel: Option<Panel>,
    pub(super) issue: Option<AccountUsageIssue>,
}

/// One automatic or manual local read: start a probe, then take Claude Code's
/// own saved result if it is from this probe, else the rendered panel.
pub(super) fn read_local(paths: &ClaudeUsagePaths) -> AccountProviderUsage {
    let Some(binary) = resolve_claude() else {
        return unavailable(AccountUsageIssue::SourceUnavailable);
    };
    if prepare_probe_dir(&paths.probe_dir).is_err() {
        return failed(AccountUsageIssue::SourceUnavailable);
    }
    let outcome = run_probe(&binary, &paths.probe_dir, &paths.config_file);
    remove_probe_transcripts(&paths.projects_dir, &paths.probe_dir);
    result_from(outcome, read_cached_usage(&paths.config_file))
}

/// A saved reading older than this is not taken as the source of a panel
/// that shows the same numbers. Claude Code shows its saved copy in place of
/// a failed fetch only while the copy is under an hour old.
const SAME_READING_WINDOW_MS: i64 = 60 * 60 * 1000;

/// Claude Code's saved reading when this probe made it fetch, or when the
/// panel shows exactly that saved reading (`/usage` shows its saved copy when
/// it fetched recently, or for up to an hour when a fetch failed): either way
/// the reading's own fetch time is the read time, never the panel's, whatever
/// its age; the one stale rule (`account_usage::stale_at`) then marks an old
/// copy stale. Otherwise the panel's numbers, read at the moment the panel
/// showed them: they differ from the saved copy, so they are what Claude Code
/// reported at that moment.
fn result_from(outcome: ProbeOutcome, cached: Option<CachedUsage>) -> AccountProviderUsage {
    let cached = match cached {
        Some(cached) if cached.fetched_at_ms >= outcome.started_ms => {
            return cached.into_usage();
        }
        other => other,
    };
    if let Some(panel) = &outcome.panel {
        let windows = parse_usage_screen(&panel.text, panel.at);
        if !windows.is_empty() {
            if let Some(cached) = cached.filter(|cached| {
                (0..SAME_READING_WINDOW_MS)
                    .contains(&(panel.at.as_millisecond() - cached.fetched_at_ms))
                    && same_numbers(&windows, &cached.windows)
            }) {
                return cached.into_usage();
            }
            let mut usage = available(windows);
            usage.checked_at = Some(panel.at.as_second());
            return usage;
        }
    }
    failed(outcome.issue.unwrap_or(AccountUsageIssue::InvalidResponse))
}

/// Which limit a window is, across the saved and on-screen spellings.
fn limit_kind(window: &AccountUsageWindow) -> Option<String> {
    match window.scope {
        AccountUsageScope::AllModels => match window.window_key.as_str() {
            "five_hour" | "session" => Some("session".into()),
            "seven_day" | "weekly_all" => Some("week".into()),
            _ => None,
        },
        AccountUsageScope::Model => Some(format!("model:{}", window.name.to_lowercase())),
        AccountUsageScope::Unspecified => None,
    }
}

/// Both sides show the session and the all-models week, with the same whole
/// percents. Model rows are left out: their labels differ between the saved
/// copy and the screen ("Sonnet only" against "Sonnet").
fn same_numbers(screen: &[AccountUsageWindow], saved: &[AccountUsageWindow]) -> bool {
    let percent = |windows: &[AccountUsageWindow], kind: &str| {
        windows
            .iter()
            .find(|window| limit_kind(window).as_deref() == Some(kind))
            .map(|window| window.used_percent.round() as i64)
    };
    ["session", "week"].iter().all(|kind| {
        let shown = percent(screen, kind);
        shown.is_some() && shown == percent(saved, kind)
    })
}

fn prepare_probe_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Claude Code's project folder name for a working directory: every
/// character other than an ASCII letter or digit becomes `-`.
pub(super) fn encoded_project_name(dir: &Path) -> String {
    // One `-` per UTF-16 unit, as Claude Code's JavaScript replacement does
    // for characters outside the Basic Multilingual Plane.
    let mut name = String::new();
    for c in dir.to_string_lossy().chars() {
        if c.is_ascii_alphanumeric() {
            name.push(c);
        } else {
            name.extend(std::iter::repeat_n('-', c.len_utf16()));
        }
    }
    name
}

/// Deletes conversation files Claude Code may have saved for the probe
/// folder, only directly inside that one project folder.
pub(super) fn remove_probe_transcripts(projects_dir: &Path, probe_dir: &Path) {
    let mut spellings = vec![probe_dir.to_path_buf()];
    if let Ok(real) = std::fs::canonicalize(probe_dir) {
        spellings.push(real);
    }
    for spelling in spellings {
        let name = encoded_project_name(&spelling);
        if !name.ends_with(&format!("-{PROBE_DIR_NAME}")) {
            continue;
        }
        let folder = projects_dir.join(name);
        let Ok(meta) = std::fs::symlink_metadata(&folder) else {
            continue;
        };
        if !meta.is_dir() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_file = entry.file_type().is_ok_and(|kind| kind.is_file());
            if is_file && path.extension().is_some_and(|ext| ext == "jsonl") {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

fn resolve_claude() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&path)
                .filter(|directory| directory.is_absolute())
                .map(|directory| directory.join("claude")),
        );
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        candidates.push(home.join(".local/bin/claude"));
        candidates.push(home.join(".claude/local/claude"));
    }
    candidates.extend([
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
    ]);
    candidates
        .into_iter()
        .find(|path| crate::account_usage_codex::is_executable(path))
}

#[cfg(not(target_os = "macos"))]
fn run_probe(_binary: &Path, _probe_dir: &Path, _config_file: &Path) -> ProbeOutcome {
    ProbeOutcome {
        started_ms: Timestamp::now().as_millisecond(),
        panel: None,
        issue: Some(AccountUsageIssue::SourceUnavailable),
    }
}

#[cfg(target_os = "macos")]
fn run_probe(binary: &Path, probe_dir: &Path, config_file: &Path) -> ProbeOutcome {
    let started_ms = Timestamp::now().as_millisecond();
    let outcome = |panel, issue| ProbeOutcome {
        started_ms,
        panel,
        issue,
    };
    if probe_dir
        .file_name()
        .is_none_or(|name| name != PROBE_DIR_NAME)
    {
        return outcome(None, Some(AccountUsageIssue::SourceUnavailable));
    }
    let args = [
        "--strict-mcp-config",
        "--allowed-tools",
        "",
        "--settings",
        SETTINGS,
    ];
    let Ok(mut session) = pty::Session::spawn(binary, probe_dir, ROWS, COLS, &args) else {
        return outcome(None, Some(AccountUsageIssue::SourceUnavailable));
    };
    let started = Instant::now();
    let (panel, issue) = interact(&mut session, started, started_ms, config_file);
    session.quit(started + HARD_DEADLINE);
    outcome(panel, issue)
}

#[cfg(target_os = "macos")]
enum Phase {
    Starting {
        ready_at: Instant,
    },
    Typed {
        at: Instant,
    },
    Submitted {
        at: Instant,
        panel_at: Option<Instant>,
    },
}

#[cfg(target_os = "macos")]
fn interact(
    session: &mut pty::Session,
    started: Instant,
    started_ms: i64,
    config_file: &Path,
) -> (Option<Panel>, Option<AccountUsageIssue>) {
    let mut screen = Screen::new(ROWS as usize, COLS as usize);
    let mut total = 0usize;
    let mut last_output = Instant::now();
    let mut last_look = started;
    let mut phase = Phase::Starting {
        ready_at: started + Duration::from_secs(2),
    };
    let mut trust_sends = 0;
    let mut last_trust_send: Option<Instant> = None;
    let mut last_cache_check = started;
    let mut panel: Option<Panel> = None;
    loop {
        if Instant::now() >= started + INTERACT_DEADLINE {
            return (panel, Some(AccountUsageIssue::Timeout));
        }
        match session.read(Duration::from_millis(50)) {
            pty::Read::Data(bytes) => {
                total += bytes.len();
                if total > MAX_OUTPUT_BYTES {
                    return (panel, Some(AccountUsageIssue::InvalidResponse));
                }
                screen.feed(&bytes);
                let replies = screen.take_replies();
                if !replies.is_empty() && session.write(&replies).is_err() {
                    return (panel, Some(AccountUsageIssue::SourceUnavailable));
                }
                last_output = Instant::now();
            }
            pty::Read::Idle => {}
            pty::Read::Closed => {
                return (panel, Some(AccountUsageIssue::SourceUnavailable));
            }
        }
        if session.exited() {
            return (panel, Some(AccountUsageIssue::SourceUnavailable));
        }
        let now = Instant::now();
        if now.duration_since(last_look) < Duration::from_millis(100) {
            continue;
        }
        last_look = now;
        // Keys are only sent to a screen that stopped changing, so a
        // half-drawn dialog is never answered.
        let settled = now.duration_since(last_output) >= Duration::from_millis(250);
        let text = screen.text();
        match trust_action(&text) {
            TrustAction::Absent => {}
            TrustAction::Wait => continue,
            TrustAction::Send(keys) => {
                let spaced = last_trust_send
                    .is_none_or(|at| now.duration_since(at) >= Duration::from_millis(400));
                if settled && trust_sends < 6 && spaced {
                    if session.write(keys).is_err() {
                        return (panel, Some(AccountUsageIssue::SourceUnavailable));
                    }
                    trust_sends += 1;
                    last_trust_send = Some(now);
                    phase = Phase::Starting {
                        ready_at: now + Duration::from_secs(2),
                    };
                }
                continue;
            }
        }
        match &mut phase {
            Phase::Starting { ready_at } => {
                // A screen that keeps animating is typed into a little later.
                let ready =
                    now >= *ready_at && (settled || now >= *ready_at + Duration::from_secs(3));
                if ready {
                    if session.write(b"/usage").is_err() {
                        return (panel, Some(AccountUsageIssue::SourceUnavailable));
                    }
                    phase = Phase::Typed { at: now };
                }
            }
            Phase::Typed { at } => {
                if now.duration_since(*at) >= Duration::from_millis(1500) {
                    if session.write(b"\r").is_err() {
                        return (panel, Some(AccountUsageIssue::SourceUnavailable));
                    }
                    phase = Phase::Submitted {
                        at: now,
                        panel_at: None,
                    };
                }
            }
            Phase::Submitted { at, panel_at } => {
                if now.duration_since(last_cache_check) >= Duration::from_millis(500) {
                    last_cache_check = now;
                    if read_cached_usage(config_file)
                        .is_some_and(|cached| cached.fetched_at_ms >= started_ms)
                    {
                        let at = Timestamp::now();
                        return (panel.or(Some(Panel { text, at })), None);
                    }
                }
                let windows = parse_usage_screen(&text, Timestamp::now());
                if windows
                    .iter()
                    .any(|w| w.scope == crate::account_usage_dto::AccountUsageScope::AllModels)
                {
                    panel = Some(Panel {
                        text,
                        at: Timestamp::now(),
                    });
                    let first = *panel_at.get_or_insert(now);
                    // Give Claude Code a moment to save its own copy, which
                    // is preferred over screen text.
                    if now.duration_since(first) >= Duration::from_millis(1500) {
                        return (panel, None);
                    }
                }
                if now.duration_since(*at) >= Duration::from_secs(12) {
                    return (panel, Some(AccountUsageIssue::Timeout));
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
mod pty {
    //! Minimal pseudo-terminal plumbing with the C library calls declared
    //! directly, as the rest of this crate does for `kill`.
    use std::{
        ffi::{c_char, c_int, c_ulong, c_void},
        fs::File,
        io::{Read as _, Write as _},
        os::fd::{AsRawFd, FromRawFd, OwnedFd},
        os::unix::process::CommandExt,
        path::Path,
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };

    #[repr(C)]
    struct Winsize {
        ws_row: u16,
        ws_col: u16,
        ws_xpixel: u16,
        ws_ypixel: u16,
    }

    #[repr(C)]
    struct PollFd {
        fd: c_int,
        events: i16,
        revents: i16,
    }

    const POLLIN: i16 = 0x1;
    const TIOCSCTTY: c_ulong = 0x2000_7461;
    /// `_IO('f', 1)`: set close-on-exec on a descriptor.
    const FIOCLEX: c_ulong = 0x2000_6601;
    const SIGKILL: c_int = 9;

    unsafe extern "C" {
        fn openpty(
            master: *mut c_int,
            slave: *mut c_int,
            name: *mut c_char,
            termp: *const c_void,
            winp: *const Winsize,
        ) -> c_int;
        fn poll(fds: *mut PollFd, nfds: u32, timeout: c_int) -> c_int;
        fn setsid() -> c_int;
        fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
        fn kill(pid: c_int, signal: c_int) -> c_int;
        fn waitid(idtype: c_int, id: u32, info: *mut c_void, options: c_int) -> c_int;
    }

    // macOS <sys/wait.h>.
    const P_PID: c_int = 1;
    const WNOHANG: c_int = 0x01;
    const WEXITED: c_int = 0x04;
    const WNOWAIT: c_int = 0x20;

    /// Room for macOS's 104-byte `siginfo_t`, 8-byte aligned.
    #[repr(C, align(8))]
    struct SigInfo([u8; 128]);
    /// Offset of `si_pid` in macOS's `siginfo_t` (after three ints).
    const SI_PID_OFFSET: usize = 12;

    /// Asks whether `pid` has exited while leaving it unreaped (`WNOWAIT`).
    fn exited_unreaped(pid: u32) -> bool {
        let mut info = SigInfo([0; 128]);
        // SAFETY: `info` is larger than and as aligned as `siginfo_t`; with
        // WNOHANG the call does not block, with WNOWAIT it reaps nothing.
        let result = unsafe {
            waitid(
                P_PID,
                pid,
                info.0.as_mut_ptr().cast(),
                WEXITED | WNOHANG | WNOWAIT,
            )
        };
        if result != 0 {
            // ECHILD: no such child any more; treat as exited.
            return true;
        }
        // With WNOHANG and no state change, si_pid stays zero.
        let mut si_pid = [0u8; 4];
        si_pid.copy_from_slice(&info.0[SI_PID_OFFSET..SI_PID_OFFSET + 4]);
        i32::from_ne_bytes(si_pid) != 0
    }

    pub(super) enum Read {
        Data(Vec<u8>),
        Idle,
        Closed,
    }

    pub(super) struct Session {
        master: File,
        child: Child,
        exited: bool,
    }

    impl Session {
        pub(super) fn spawn(
            binary: &Path,
            cwd: &Path,
            rows: u16,
            cols: u16,
            args: &[&str],
        ) -> std::io::Result<Self> {
            let size = Winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            };
            let (mut master, mut slave) = (-1, -1);
            // SAFETY: valid out-pointers and a valid window size; no name or
            // termios is requested.
            let opened = unsafe {
                openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null(),
                    &size,
                )
            };
            if opened != 0 {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: openpty returned two new descriptors this function owns.
            let (master, slave) =
                unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) };
            // Close-on-exec at once, so neither side leaks into the child
            // (beyond the standard streams set up below) or into any process
            // another thread starts.
            for fd in [&master, &slave] {
                // SAFETY: a flag change on a descriptor this function owns.
                if unsafe { ioctl(fd.as_raw_fd(), FIOCLEX) } == -1 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            let mut command = Command::new(binary);
            command
                .args(args)
                .current_dir(cwd)
                .stdin(Stdio::from(slave.try_clone()?))
                .stdout(Stdio::from(slave.try_clone()?))
                .stderr(Stdio::from(slave));
            configure_environment(&mut command, binary);
            // SAFETY: setsid and ioctl are async-signal-safe. The child gets
            // its own session with the pseudo-terminal (already its stdin) as
            // controlling terminal, so closing the master hangs it up and its
            // process group can be killed as a whole.
            unsafe {
                command.pre_exec(|| {
                    if setsid() == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    if ioctl(0, TIOCSCTTY, 0) == -1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let child = command.spawn()?;
            // The command (and with it every parent copy of the slave side) is
            // dropped here, so a quitting child closes the terminal.
            drop(command);
            Ok(Self {
                master: File::from(master),
                child,
                exited: false,
            })
        }

        pub(super) fn read(&mut self, wait: Duration) -> Read {
            let mut fds = PollFd {
                fd: self.master.as_raw_fd(),
                events: POLLIN,
                revents: 0,
            };
            let millis = c_int::try_from(wait.as_millis()).unwrap_or(c_int::MAX);
            // SAFETY: one valid pollfd for an open descriptor.
            let ready = unsafe { poll(&mut fds, 1, millis) };
            if ready <= 0 {
                return Read::Idle;
            }
            let mut buffer = vec![0u8; 16 * 1024];
            match self.master.read(&mut buffer) {
                Ok(0) | Err(_) => Read::Closed,
                Ok(count) => {
                    buffer.truncate(count);
                    Read::Data(buffer)
                }
            }
        }

        pub(super) fn write(&mut self, bytes: &[u8]) -> std::io::Result<()> {
            self.master.write_all(bytes)?;
            self.master.flush()
        }

        /// Whether the child has exited, without reaping it: until `quit`
        /// reaps it, its pid (and so its process group id) cannot be reused,
        /// so the group can always be signalled safely.
        pub(super) fn exited(&mut self) -> bool {
            if !self.exited {
                self.exited = exited_unreaped(self.child.id());
            }
            self.exited
        }

        /// Dismisses the panel, asks Claude Code to exit, and kills the whole
        /// process group if it is still running at `deadline`.
        pub(super) fn quit(mut self, deadline: Instant) {
            if !self.exited() {
                let _ = self.write(b"\x1b");
                self.drain(Duration::from_millis(300));
                let _ = self.write(b"/exit");
                self.drain(Duration::from_millis(400));
                let _ = self.write(b"\r");
                let polite = (Instant::now() + Duration::from_secs(4)).min(deadline);
                while Instant::now() < polite && !self.exited() {
                    self.drain(Duration::from_millis(50));
                }
            }
            // The child is not reaped yet (`exited` never reaps), so its pid
            // and group id are still its own: end every helper left in the
            // group, whether or not Claude Code itself quit, then reap.
            if let Ok(pid) = c_int::try_from(self.child.id()) {
                // SAFETY: signals the child's own, still unreaped, process
                // group (its session id equals its pid).
                unsafe { kill(-pid, SIGKILL) };
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }

        fn drain(&mut self, wait: Duration) {
            let until = Instant::now() + wait;
            while Instant::now() < until {
                if matches!(self.read(Duration::from_millis(25)), Read::Closed) {
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
    }

    fn configure_environment(command: &mut Command, binary: &Path) {
        // Finder starts apps with a short PATH; Claude Code may need node or
        // its own helpers beside it.
        let mut path: Vec<std::path::PathBuf> = Vec::new();
        if let Some(parent) = binary.parent() {
            path.push(parent.to_path_buf());
        }
        if let Some(current) = std::env::var_os("PATH") {
            path.extend(std::env::split_paths(&current));
        }
        for extra in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"] {
            let extra = std::path::PathBuf::from(extra);
            if !path.contains(&extra) {
                path.push(extra);
            }
        }
        if let Ok(joined) = std::env::join_paths(path) {
            command.env("PATH", joined);
        }
        command.env("TERM", "xterm-256color");
        command.env("DISABLE_AUTOUPDATER", "1");
        if std::env::var_os("LANG").is_none() {
            command.env("LANG", "en_US.UTF-8");
        }
        command.env_remove("COLUMNS");
        command.env_remove("LINES");
        // A probe reads the signed-in subscription's limits; an API key or a
        // parent Claude Code session in the environment must not change that.
        for (key, _) in std::env::vars_os() {
            let Some(key) = key.to_str() else { continue };
            if key.starts_with("ANTHROPIC_")
                || key == "CLAUDECODE"
                || key == "CLAUDE_CODE_ENTRYPOINT"
                || key == "CLAUDE_CODE_SSE_PORT"
            {
                command.env_remove(key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account_usage_dto::AccountUsageState;

    fn window(percent: f64) -> AccountUsageWindow {
        AccountUsageWindow {
            bucket_key: "seven_day".into(),
            window_key: "seven_day".into(),
            scope: AccountUsageScope::AllModels,
            name: "Claude".into(),
            window: "Week".into(),
            used_percent: percent,
            duration_minutes: Some(10080),
            resets_at: None,
            pace: None,
            daily: None,
            series: None,
        }
    }

    #[test]
    fn project_folder_name_matches_claude_code_encoding() {
        assert_eq!(
            encoded_project_name(Path::new(
                "/Users/a b/Library/Application Support/ai.xtrace.desktop/xtrace-claude-usage-probe"
            )),
            "-Users-a-b-Library-Application-Support-ai-xtrace-desktop-xtrace-claude-usage-probe"
        );
        // Outside the Basic Multilingual Plane: two UTF-16 units, two dashes.
        assert_eq!(encoded_project_name(Path::new("/a/🎉/é")), "-a-----");
    }

    #[test]
    fn removes_only_probe_folder_transcripts() {
        let root = tempfile::tempdir().unwrap();
        let projects = root.path().join("projects");
        let probe = root.path().join("data").join(PROBE_DIR_NAME);
        std::fs::create_dir_all(&probe).unwrap();
        let probe_project = projects.join(encoded_project_name(&probe));
        let real_project = projects.join(encoded_project_name(
            &std::fs::canonicalize(&probe).unwrap(),
        ));
        let other = projects.join("-Users-someone-repo");
        for dir in [&probe_project, &real_project, &other] {
            std::fs::create_dir_all(dir.join("memory")).unwrap();
            std::fs::write(dir.join("s.jsonl"), "{}").unwrap();
            std::fs::write(dir.join("notes.txt"), "keep").unwrap();
            std::fs::write(dir.join("memory").join("m.jsonl"), "{}").unwrap();
        }
        remove_probe_transcripts(&projects, &probe);
        for dir in [&probe_project, &real_project] {
            assert!(!dir.join("s.jsonl").exists());
            assert!(dir.join("notes.txt").exists());
            assert!(dir.join("memory").join("m.jsonl").exists());
        }
        assert!(other.join("s.jsonl").exists());
        // A folder of a different name is never touched.
        remove_probe_transcripts(&projects, &root.path().join("repo"));
        assert!(other.join("s.jsonl").exists());
    }

    fn panel(text: &str, at_ms: i64) -> Option<Panel> {
        Some(Panel {
            text: text.to_owned(),
            at: Timestamp::from_millisecond(at_ms).unwrap(),
        })
    }

    fn cached(ms: i64, percent: f64) -> Option<CachedUsage> {
        Some(CachedUsage {
            fetched_at_ms: ms,
            windows: vec![window(percent)],
        })
    }

    const PANEL_MS: i64 = 1_790_882_872_000;
    const STARTED_MS: i64 = PANEL_MS - 5_000;

    fn outcome(panel: Option<Panel>, issue: Option<AccountUsageIssue>) -> ProbeOutcome {
        ProbeOutcome {
            started_ms: STARTED_MS,
            panel,
            issue,
        }
    }

    #[test]
    fn saved_result_from_this_probe_wins_and_failures_keep_their_issue() {
        let fresh = result_from(outcome(None, None), cached(STARTED_MS + 500, 37.0));
        assert_eq!(fresh.state, AccountUsageState::Available);
        assert_eq!(fresh.checked_at, Some((STARTED_MS + 500) / 1000));
        let old = result_from(
            outcome(None, Some(AccountUsageIssue::Timeout)),
            cached(STARTED_MS - 1, 37.0),
        );
        assert_eq!(
            (old.state, old.issue),
            (AccountUsageState::Failed, Some(AccountUsageIssue::Timeout))
        );
        let neither = result_from(outcome(panel("nothing", PANEL_MS), None), None);
        assert_eq!(neither.issue, Some(AccountUsageIssue::InvalidResponse));
    }

    fn session(key: &str, percent: f64) -> AccountUsageWindow {
        AccountUsageWindow {
            bucket_key: key.into(),
            window_key: key.into(),
            window: "Session".into(),
            ..window(percent)
        }
    }

    fn model(name: &str, percent: f64) -> AccountUsageWindow {
        AccountUsageWindow {
            scope: AccountUsageScope::Model,
            name: name.into(),
            window_key: "weekly_scoped".into(),
            ..window(percent)
        }
    }

    fn saved(ms: i64, session_percent: f64, week_percent: f64) -> Option<CachedUsage> {
        Some(CachedUsage {
            fetched_at_ms: ms,
            windows: vec![
                session("session", session_percent),
                AccountUsageWindow {
                    window_key: "weekly_all".into(),
                    ..window(week_percent)
                },
            ],
        })
    }

    #[test]
    fn panel_showing_the_saved_reading_keeps_its_real_fetch_time() {
        let text = "Current session\n██ 30% used\nCurrent week (all models)\n██ 41% used\n\
                    Current week (Sonnet only)\n██ 12% used";
        let saved_at = PANEL_MS - 45_000;
        let at = |cached| result_from(outcome(panel(text, PANEL_MS), None), cached).checked_at;
        // Same session and week: the panel showed the saved copy, whose own
        // fetch time is used, even though its model rows differ (a label
        // spelled differently, an extra limit).
        let mut same = saved(saved_at, 30.0, 41.0).unwrap();
        same.windows.push(model("Sonnet", 12.0));
        same.windows.push(model("Fable", 19.0));
        let used = result_from(outcome(panel(text, PANEL_MS), None), Some(same));
        assert_eq!(used.checked_at, Some(saved_at / 1000));
        assert!(used.windows.iter().any(|w| w.name == "Fable"));
        // Different session or week: the panel's own numbers, read when it
        // showed them.
        assert_eq!(at(saved(saved_at, 31.0, 41.0)), Some(PANEL_MS / 1000));
        assert_eq!(at(saved(saved_at, 30.0, 40.0)), Some(PANEL_MS / 1000));
        let differs = result_from(
            outcome(panel(text, PANEL_MS), None),
            saved(saved_at, 30.0, 40.0),
        );
        assert!(differs.windows.iter().any(|w| w.used_percent == 41.0));
        // No saved copy, or one too old to be the panel's source.
        assert_eq!(at(None), Some(PANEL_MS / 1000));
        assert_eq!(
            at(saved(PANEL_MS - SAME_READING_WINDOW_MS, 30.0, 41.0)),
            Some(PANEL_MS / 1000)
        );
        // A side missing the session (or the week) cannot be matched.
        let week_only = cached(saved_at, 41.0);
        assert_eq!(at(week_only), Some(PANEL_MS / 1000));
    }

    /// A panel showing Claude Code's saved copy keeps the copy's own fetch
    /// time, however old, never the panel's time; the stale rule then marks
    /// an old copy stale. Such a read succeeds.
    #[test]
    fn panel_showing_an_old_saved_reading_keeps_its_own_time() {
        let text = "Current session\n██ 30% used\nCurrent week (all models)\n██ 41% used";
        for age in [crate::account_usage::STALE_AFTER_SECS - 1, 50 * 60] {
            let read = result_from(
                outcome(panel(text, PANEL_MS), None),
                saved(PANEL_MS - age * 1000, 30.0, 41.0),
            );
            assert_eq!(read.state, AccountUsageState::Available);
            assert_eq!(read.checked_at, Some(PANEL_MS / 1000 - age));
        }
    }

    #[test]
    fn saved_and_screen_spellings_of_the_session_and_week_match() {
        let weekly_all = AccountUsageWindow {
            window_key: "weekly_all".into(),
            ..window(39.0)
        };
        let screen = [
            session("five_hour", 30.0),
            window(39.0),
            model("Sonnet only", 5.0),
        ];
        let saved = [
            session("session", 30.0),
            weekly_all.clone(),
            model("Sonnet", 6.0),
        ];
        assert!(same_numbers(&screen, &saved));
        assert!(same_numbers(&screen, &saved[..2]));
        // Both the session and the week must be on both sides.
        assert!(!same_numbers(&screen[1..], &saved[1..]));
        assert!(!same_numbers(&screen, &[weekly_all]));
        assert!(!same_numbers(
            &[session("five_hour", 31.0), window(39.0)],
            &saved
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn quitting_ends_helpers_left_in_the_group() {
        let dir = tempfile::tempdir().unwrap();
        // The helper ignores the terminal hangup, as a detached helper could,
        // so only the group kill can end it.
        let mut session = pty::Session::spawn(
            Path::new("/bin/sh"),
            dir.path(),
            ROWS,
            COLS,
            &["-c", "trap '' HUP; sleep 30 & echo helper=$!; exit 0"],
        )
        .unwrap();
        let mut screen = Screen::new(ROWS as usize, COLS as usize);
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until && !session.exited() {
            if let pty::Read::Data(bytes) = session.read(Duration::from_millis(50)) {
                screen.feed(&bytes);
            }
        }
        assert!(session.exited(), "the shell did not exit");
        let text = screen.text();
        let helper: i32 = text
            .split("helper=")
            .nth(1)
            .and_then(|rest| rest.split_whitespace().next())
            .and_then(|pid| pid.parse().ok())
            .unwrap_or_else(|| panic!("no helper pid in {text:?}"));
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        // SAFETY: signal 0 only checks that the process exists.
        assert_eq!(unsafe { kill(helper, 0) }, 0, "helper not running");
        session.quit(Instant::now() + Duration::from_secs(5));
        let gone_by = Instant::now() + Duration::from_secs(3);
        // SAFETY: as above.
        while unsafe { kill(helper, 0) } == 0 && Instant::now() < gone_by {
            std::thread::sleep(Duration::from_millis(20));
        }
        // SAFETY: as above.
        assert_eq!(unsafe { kill(helper, 0) }, -1, "helper outlived quit");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn child_sees_only_its_terminal_streams() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = pty::Session::spawn(
            Path::new("/bin/sh"),
            dir.path(),
            ROWS,
            COLS,
            &[
                "-c",
                "for fd in 3 4 5 6 7 8 9; do (: <&$fd) 2>/dev/null && echo open-$fd; done; echo checked; sleep 0.2",
            ],
        )
        .unwrap();
        let mut screen = Screen::new(ROWS as usize, COLS as usize);
        let until = Instant::now() + Duration::from_secs(5);
        while Instant::now() < until {
            match session.read(Duration::from_millis(50)) {
                pty::Read::Data(bytes) => screen.feed(&bytes),
                pty::Read::Closed => break,
                pty::Read::Idle if session.exited() => break,
                pty::Read::Idle => {}
            }
        }
        session.quit(Instant::now() + Duration::from_secs(2));
        let text = screen.text();
        // The shell itself holds only the terminal as 0-2. (The shell's own
        // redirection saves a stream at 10, so the check stops at 9.)
        assert!(text.contains("checked"), "{text}");
        assert!(!text.contains("open-"), "{text}");
    }

    /// One real probe against the installed Claude Code. It sends no message.
    /// Run with: XTRACE_LIVE_CLAUDE_PROBE=1 cargo test -p xtrace-desktop
    ///   live_claude_usage_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn live_claude_usage_probe() {
        if std::env::var_os("XTRACE_LIVE_CLAUDE_PROBE").is_none() {
            return;
        }
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        let data = std::env::var_os("XTRACE_LIVE_PROBE_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("xtrace-live-probe-data"));
        let paths = ClaudeUsagePaths::new(&data, &home, None);
        let started = Instant::now();
        let binary = resolve_claude().expect("claude is installed");
        prepare_probe_dir(&paths.probe_dir).unwrap();
        let outcome = run_probe(&binary, &paths.probe_dir, &paths.config_file);
        remove_probe_transcripts(&paths.projects_dir, &paths.probe_dir);
        println!(
            "elapsed: {:?}, issue: {:?}",
            started.elapsed(),
            outcome.issue
        );
        if let Some(panel) = &outcome.panel {
            println!(
                "--- panel at {} ---\n{}\n--- screen parse ---",
                panel.at, panel.text
            );
            for window in parse_usage_screen(&panel.text, panel.at) {
                println!("{window:?}");
            }
        }
        let usage = result_from(outcome, read_cached_usage(&paths.config_file));
        println!("{}", serde_json::to_string_pretty(&usage).unwrap());
        let project = paths
            .projects_dir
            .join(encoded_project_name(&paths.probe_dir));
        println!(
            "probe project folder {} exists: {}",
            project.display(),
            project.exists()
        );
        if let Ok(entries) = std::fs::read_dir(&project) {
            for entry in entries.flatten() {
                println!("  left in project folder: {}", entry.path().display());
            }
        }
    }
}
