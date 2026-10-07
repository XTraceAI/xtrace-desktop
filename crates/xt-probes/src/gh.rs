//! The GitHub CLI pull-request view: one bounded `gh pr view` attempt for one
//! already validated [`PrIdentity`], turned into the storage layer's typed
//! [`RefreshOutcome`].
//!
//! The shape of this client is its guarantee.
//!
//! * It runs exactly one program, the resolved absolute executable the caller
//!   supplied, with exactly one argument vector:
//!   `gh pr view <canonical url> --json number,title,url,state,mergedAt,additions,deletions,headRefName`.
//!   There is no shell, no interpolation, no flag built from a response, and
//!   no second command. The canonical URL comes from [`PrIdentity::url`], so
//!   the argument cannot be anything a caller spelled by hand.
//! * It never infers a repository from a working directory. The child runs
//!   with `/` as its directory and without `GH_REPO`, so the only pull request
//!   it can name is the one in the argument.
//! * Its child has no standard input (`/dev/null`), no interactive prompt, no
//!   pager, no spinner, no colour and no update notice. Every one of those is
//!   set on the child's environment, never read back from the parent's.
//! * The credential environment is preserved for the child exactly as this
//!   process received it: `GH_TOKEN`, `GITHUB_TOKEN`, `HOME` and the rest are
//!   inherited untouched. This module never reads, copies, logs, returns or
//!   stores any of them, and never runs a login, auth or credential command.
//! * Both output streams are separately bounded and drained concurrently, so
//!   a child that floods one of them can neither exhaust memory here nor
//!   block on a full pipe. Stdout past [`Limits::max_stdout_bytes`] ends the
//!   attempt as `output_too_large`; stderr past [`Limits::max_stderr_bytes`]
//!   is discarded. Stderr is never reported or stored; it is only compared,
//!   whole, with the one "pull request does not exist" line below.
//! * A deadline, a cancellation and an oversized stdout all terminate the
//!   child's whole process group and reap it, so a descendant that inherited
//!   the pipes cannot keep the attempt alive. The child's exit is observed
//!   without being consumed, so the leader stays unreaped until that decision
//!   has been made: its pid is what reserves the ID of the group a
//!   termination signals, and it is given up exactly once, afterwards.
//! * Failures are the conservative closed codes of [`PrRefreshError`]. A
//!   process that ran and exited nonzero is `execution_failed`, whatever it
//!   wrote, with two exceptions:
//!   - exit code 4, which `gh help exit-codes` documents as "authentication
//!     required", is `unauthorized`. That code is read from the exit status
//!     alone.
//!   - exit code 1 whose whole stderr is exactly the GraphQL answer that the
//!     repository resolved but has no pull request with the requested number
//!     is `not_found` ([`pull_request_missing_line`]). gh 2.90.0 prints, for
//!     a visible repository without that pull request:
//!     `GraphQL: Could not resolve to a PullRequest with the number of 999. (repository.pullRequest)`
//!     The `(repository.pullRequest)` path says the `repository` field
//!     resolved and only `pullRequest` did not. A repository gh cannot see —
//!     missing, or private to another account — fails one level up instead,
//!     `GraphQL: Could not resolve to a Repository with the name 'x/r'. (repository)`,
//!     and stays `execution_failed`, as does any other wording, extra line or
//!     truncated stderr.
//!
//!   This client never produces "rate limited"; that code exists in storage
//!   for an owner that can establish it.
//! * A response is accepted only in its exact documented shape, and only when
//!   it names the pull request that was requested. Field rules are not
//!   restated here: the parsed result is checked by
//!   [`RefreshOutcome::validate`], the same validation
//!   `Store::record_pr_refresh` applies before it writes.
//!
//! Nothing here schedules, decides staleness, or writes to a database. One
//! call is one attempt, stamped with one reading of the caller's clock.
//!
//! Only Unix hosts are supported: the bounded-termination guarantee is built
//! on process groups. [`GhClient::new`] reports
//! [`GhClientError::UnsupportedPlatform`] anywhere else rather than running a
//! process it could not contain.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use xt_store::pr_link::{
    MAX_ATTEMPTED_AT, PrIdentity, PrRefreshError, PrState, RefreshFailure, RefreshOutcome,
    RefreshSuccess,
};

/// The JSON fields requested, in the exact order of the argument.
pub const JSON_FIELDS: &str = "number,title,url,state,mergedAt,additions,deletions,headRefName";
/// How long one attempt may take, from spawn to drained output.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(30);
/// Largest accepted response. More than this ends the attempt.
pub const DEFAULT_MAX_STDOUT_BYTES: usize = 1024 * 1024;
/// Largest retained diagnostic output. More than this is discarded unread.
pub const DEFAULT_MAX_STDERR_BYTES: usize = 64 * 1024;

/// The bounds of one attempt. Tests shorten them; the defaults are the
/// documented ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub deadline: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            deadline: DEFAULT_DEADLINE,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            max_stderr_bytes: DEFAULT_MAX_STDERR_BYTES,
        }
    }
}

/// A refusal to attempt at all. These are not refresh results: nothing ran,
/// so there is no attempt to record. Every condition reachable once a child
/// has been started is a typed [`PrRefreshError`] instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GhClientError {
    /// This platform has no supported bounded-termination boundary.
    UnsupportedPlatform,
    /// The executable must be an already resolved absolute path; this client
    /// searches no `PATH` and joins no working directory.
    ExecutableNotAbsolute,
    /// A bound was zero, so no attempt could satisfy it.
    InvalidLimits,
    /// The clock read an attempt time storage cannot hold.
    AttemptTimeOutOfRange,
}

impl std::fmt::Display for GhClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self {
            Self::UnsupportedPlatform => "the GitHub CLI client supports Unix hosts only",
            Self::ExecutableNotAbsolute => "the GitHub CLI executable must be an absolute path",
            Self::InvalidLimits => "the GitHub CLI client bounds must be nonzero",
            Self::AttemptTimeOutOfRange => "the attempt time is outside the recordable range",
        };
        f.write_str(reason)
    }
}

impl std::error::Error for GhClientError {}

/// The caller's clock, read once per attempt. The reading stamps the stored
/// result, so it is supplied rather than taken here: a caller that orders
/// attempts owns the ordering.
pub trait AttemptClock {
    /// UTC milliseconds.
    fn attempted_at(&self) -> i64;
}

/// The host clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl AttemptClock for SystemClock {
    fn attempted_at(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|since| i64::try_from(since.as_millis()).ok())
            .unwrap_or(-1)
    }
}

/// Cancels an attempt from another thread. A cancel before the spawn refuses
/// to start the child; a cancel while it runs kills its process group and
/// reaps it. Cancellation is permanent for the flag.
///
/// This is deliberately smaller than the ingest reader's `CancelToken`: this
/// client awaits its own single child in the calling thread, so a flag the
/// wait loop polls is enough and no registry of live children is needed.
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// One `gh` executable with one set of bounds.
#[derive(Clone, Debug)]
pub struct GhClient {
    executable: PathBuf,
    limits: Limits,
}

impl GhClient {
    /// A client for an already resolved absolute executable path, with the
    /// documented bounds.
    pub fn new(executable: impl Into<PathBuf>) -> Result<Self, GhClientError> {
        Self::with_limits(executable, Limits::default())
    }

    /// A client with explicit bounds.
    pub fn with_limits(
        executable: impl Into<PathBuf>,
        limits: Limits,
    ) -> Result<Self, GhClientError> {
        if !cfg!(unix) {
            return Err(GhClientError::UnsupportedPlatform);
        }
        let executable = executable.into();
        if !executable.is_absolute() {
            return Err(GhClientError::ExecutableNotAbsolute);
        }
        if limits.deadline.is_zero() || limits.max_stdout_bytes == 0 || limits.max_stderr_bytes == 0
        {
            return Err(GhClientError::InvalidLimits);
        }
        Ok(Self { executable, limits })
    }

    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// The exact argument vector for one identity, `argv[0]` first. Nothing
    /// else is ever passed to the executable.
    pub fn argv(&self, identity: &PrIdentity) -> Vec<OsString> {
        vec![
            self.executable.clone().into_os_string(),
            OsString::from("pr"),
            OsString::from("view"),
            OsString::from(identity.url()),
            OsString::from("--json"),
            OsString::from(JSON_FIELDS),
        ]
    }

    /// Run one attempt for one identity and report its typed result.
    ///
    /// The clock is read once, before the spawn, and stamps whatever comes
    /// back. Every outcome names the identity that was requested, never one
    /// read out of the response.
    pub fn pr_view(
        &self,
        identity: &PrIdentity,
        clock: &dyn AttemptClock,
        cancel: Option<&CancelFlag>,
    ) -> Result<RefreshOutcome, GhClientError> {
        let attempted_at = clock.attempted_at();
        if !(0..=MAX_ATTEMPTED_AT).contains(&attempted_at) {
            return Err(GhClientError::AttemptTimeOutOfRange);
        }
        #[cfg(unix)]
        {
            Ok(unix::attempt(self, identity, attempted_at, cancel))
        }
        #[cfg(not(unix))]
        {
            let _ = (identity, cancel);
            Err(GhClientError::UnsupportedPlatform)
        }
    }
}

fn failure(identity: &PrIdentity, attempted_at: i64, error: PrRefreshError) -> RefreshOutcome {
    RefreshOutcome::Failure(RefreshFailure {
        pull_request: identity.clone(),
        attempted_at,
        error,
    })
}

/// One `gh pr view` response, in exactly its documented shape. Every field is
/// required and typed; an unknown field, a missing one, a string where a
/// number belongs or a state outside the closed set is not this response.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct PrView {
    number: u64,
    title: String,
    url: String,
    state: PrState,
    /// Present, and null only when the pull request is not merged.
    #[serde(deserialize_with = "present")]
    merged_at: Option<String>,
    additions: i64,
    deletions: i64,
    head_ref_name: String,
}

/// Require the key, and accept an explicit null as its value.
///
/// The field was named in the request, so a response that omits it is not the
/// response that was asked for. Serde would otherwise fill a missing
/// `Option` field with `None` and let a silently absent `mergedAt` pass as a
/// pull request that is not merged; naming a `deserialize_with` makes its
/// absence the missing-field error it should be. Whether a null is allowed
/// for the reported state stays where every other field rule is, in
/// [`RefreshOutcome::validate`].
fn present<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    <Option<String> as serde::Deserialize>::deserialize(deserializer)
}

/// How a completed run exited, judged by its exit status alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Exited {
    Success,
    /// Exit code 4: `gh help exit-codes` documents it as "authentication
    /// required".
    NeedsAuthentication,
    /// Exit code 1, gh's ordinary failure.
    Error,
    /// Any other nonzero exit, or a death by signal.
    Failed,
}

/// gh's documented exit code for "authentication required".
const GH_EXIT_AUTH: i32 = 4;
/// gh's exit code for a command that failed (`gh help exit-codes`).
const GH_EXIT_ERROR: i32 = 1;

/// The exact stderr line gh writes when the repository resolved but has no
/// pull request with this number (observed with gh 2.90.0 against a visible
/// repository):
///
/// `GraphQL: Could not resolve to a PullRequest with the number of 999. (repository.pullRequest)`
///
/// A repository that does not resolve — missing, or private to an account
/// other than the signed-in one — is reported at `(repository)` instead and
/// never matches.
pub fn pull_request_missing_line(number: u64) -> String {
    format!(
        "GraphQL: Could not resolve to a PullRequest with the number of {number}. (repository.pullRequest)"
    )
}

/// Whether a failed run's stderr is exactly gh's "this repository has no such
/// pull request" answer for the requested number, and nothing else. A
/// truncated stderr is never judged.
fn says_pull_request_missing(identity: &PrIdentity, stderr: &Stderr) -> bool {
    !stderr.overflowed
        && std::str::from_utf8(&stderr.kept).is_ok_and(|text| {
            text.trim_end_matches(['\n', '\r']) == pull_request_missing_line(identity.number())
        })
}

/// What a run wrote to stderr, kept only to compare with
/// [`pull_request_missing_line`]; never reported or stored.
#[derive(Default)]
struct Stderr {
    kept: Vec<u8>,
    /// More was written than the bound kept.
    overflowed: bool,
}

/// Turn one completed run into a typed outcome. Everything a response must
/// satisfy beyond its own shape and identity is the storage layer's
/// validation, not a second copy of those rules.
fn interpret(
    identity: &PrIdentity,
    attempted_at: i64,
    exited: Exited,
    stdout: &[u8],
    stderr: &Stderr,
) -> RefreshOutcome {
    match exited {
        Exited::Success => {}
        // gh's documented exit code for a command that needs authentication.
        Exited::NeedsAuthentication => {
            return failure(identity, attempted_at, PrRefreshError::Unauthorized);
        }
        // The repository resolved and GitHub says it has no pull request
        // with this number.
        Exited::Error if says_pull_request_missing(identity, stderr) => {
            return failure(identity, attempted_at, PrRefreshError::NotFound);
        }
        // The process ran and refused. Which refusal it was is not decidable
        // from its output here, so it is reported as one execution failure.
        Exited::Error | Exited::Failed => {
            return failure(identity, attempted_at, PrRefreshError::ExecutionFailed);
        }
    }
    let invalid = || failure(identity, attempted_at, PrRefreshError::InvalidResponse);
    let Ok(text) = std::str::from_utf8(stdout) else {
        return invalid();
    };
    let Ok(view) = serde_json::from_str::<PrView>(text) else {
        return invalid();
    };
    // The response must name the pull request that was asked for. Reconciling
    // through the canonical parser accepts only a canonical URL and requires
    // the number to agree with it.
    let Ok(reported) = PrIdentity::reconcile(Some(&view.url), None, Some(view.number)) else {
        return invalid();
    };
    if reported != *identity {
        return invalid();
    }
    let outcome = RefreshOutcome::Success(RefreshSuccess {
        pull_request: identity.clone(),
        attempted_at,
        title: view.title,
        state: view.state,
        merged_at: view.merged_at,
        additions: view.additions,
        deletions: view.deletions,
        head_ref_name: view.head_ref_name,
    });
    if outcome.validate().is_err() {
        return invalid();
    }
    outcome
}

#[cfg(unix)]
mod unix {
    use super::{CancelFlag, GhClient, PrIdentity, PrRefreshError, RefreshOutcome, failure};
    use std::{
        io::Read,
        process::{Child, Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::JoinHandle,
        time::{Duration, Instant},
    };

    /// How often the attempt is polled while the child runs.
    const POLL: Duration = Duration::from_millis(10);
    /// How long a terminated group is given to close the pipes it holds
    /// before its drains are abandoned. The child itself is always reaped.
    const DRAIN_GRACE: Duration = Duration::from_secs(5);
    /// Read size; unrelated to the retained bounds.
    const CHUNK: usize = 8 * 1024;

    pub(super) fn attempt(
        client: &GhClient,
        identity: &PrIdentity,
        attempted_at: i64,
        cancel: Option<&CancelFlag>,
    ) -> RefreshOutcome {
        if cancel.is_some_and(CancelFlag::is_cancelled) {
            return failure(identity, attempted_at, PrRefreshError::Cancelled);
        }
        let argv = client.argv(identity);
        let mut command = Command::new(&argv[0]);
        command.args(&argv[1..]);
        harden(&mut command);
        // An executable that cannot be started is not a failed GitHub call.
        let Ok(child) = spawn_grouped(&mut command) else {
            return failure(identity, attempted_at, PrRefreshError::Unavailable);
        };
        match collect(child, client.limits(), cancel) {
            Err(error) => failure(identity, attempted_at, error),
            Ok((exited, stdout, stderr)) => {
                super::interpret(identity, attempted_at, exited, &stdout, &stderr)
            }
        }
    }

    /// Everything the child may not do. The inherited environment is left
    /// otherwise untouched, so credentials reach `gh` without being read
    /// here; only the variables that would make it interactive, noisy or
    /// repository-inferring are set or removed.
    fn harden(command: &mut Command) {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // Never the caller's repository: with no inferable checkout and
            // no GH_REPO, the URL argument is the only pull request in play.
            .current_dir("/")
            .env("GH_PROMPT_DISABLED", "1")
            .env("GH_NO_UPDATE_NOTIFIER", "1")
            .env("GH_NO_EXTENSION_UPDATE_NOTIFIER", "1")
            .env("GH_SPINNER_DISABLED", "1")
            .env("GH_PAGER", "")
            .env("PAGER", "")
            .env("NO_COLOR", "1")
            .env("CLICOLOR", "0")
            .env_remove("CLICOLOR_FORCE")
            .env_remove("GH_FORCE_TTY")
            .env_remove("GH_DEBUG")
            .env_remove("DEBUG")
            .env_remove("GH_REPO")
            .env_remove("GH_MDWIDTH")
            .env_remove("GLAMOUR_STYLE");
    }

    /// Start the child as the leader of its own process group, so a deadline
    /// or cancel can terminate everything it spawned.
    fn spawn_grouped(command: &mut Command) -> std::io::Result<Child> {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
        command.spawn()
    }

    /// Whether the child has terminated, observed **without reaping it**.
    ///
    /// This is the whole reason the attempt does not poll with `try_wait`:
    /// that reaps the leader the moment it exits, which releases its pid, and
    /// the pid of a group leader is what reserves that group's ID. A launcher
    /// that exits while a descendant still holds the pipes would then leave
    /// this client holding an ID it no longer owns. `WNOWAIT` leaves the
    /// leader waitable — a zombie the kernel keeps, and a member of its own
    /// group — so the ID stays reserved until [`terminate`] or the final reap
    /// gives it up, exactly once.
    fn exit_observed(pid: u32) -> std::io::Result<bool> {
        let id = libc::id_t::try_from(pid)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
        // Zeroed first: with `WNOHANG` a report of no state change is a zero
        // `si_pid`, and not every implementation writes that zero itself.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: `info` is a valid, initialized siginfo_t for this call, and
        // the options are the POSIX non-blocking, non-consuming pair.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                id,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            )
        };
        if result == -1 {
            let error = std::io::Error::last_os_error();
            return match error.raw_os_error() {
                // An interrupted observation is simply not an observation.
                Some(libc::EINTR) => Ok(false),
                _ => Err(error),
            };
        }
        Ok(reported_pid(&info) != 0)
    }

    /// `si_pid` is a plain field on the BSDs and an accessor on Linux.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    fn reported_pid(info: &libc::siginfo_t) -> libc::pid_t {
        // SAFETY: the union is read as the child-status member the `WEXITED`
        // report fills, and as the zeroes it was initialized with otherwise.
        unsafe { info.si_pid() }
    }

    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    fn reported_pid(info: &libc::siginfo_t) -> libc::pid_t {
        info.si_pid
    }

    /// Kill every process in the group the child leads, then reap the leader
    /// exactly once.
    ///
    /// This is only ever reached with the leader still unreaped: running, or
    /// exited and kept waitable by [`exit_observed`]. Its pid is therefore
    /// still allocated and still names this group, so `-pid` cannot reach any
    /// other group, and the group is never empty at this point — even a
    /// zombie leader is a member of it. The signal is sent deliberately when
    /// the leader has already exited, because that is exactly the case where
    /// a descendant is still holding the pipes and terminating the leader
    /// alone would leave it running. The reap comes after the signal, and is
    /// the only one: it is what finally releases the pid and the group ID.
    fn terminate(pid: u32, child: &mut Child) {
        if let Ok(pid) = i32::try_from(pid) {
            // SAFETY: a plain signal to the unreaped child's own group.
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        let _ = child.wait();
    }

    /// One bounded, concurrently drained stream. The thread reads to the end
    /// whatever the bound, so the child can never block on a full pipe;
    /// bytes past the bound are dropped instead of retained.
    struct Drain {
        handle: Option<JoinHandle<Vec<u8>>>,
        overflowed: Arc<AtomicBool>,
    }

    impl Drain {
        fn spawn(mut stream: impl Read + Send + 'static, limit: usize) -> Self {
            let overflowed = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&overflowed);
            let handle = std::thread::spawn(move || {
                let mut kept: Vec<u8> = Vec::new();
                let mut buffer = [0u8; CHUNK];
                let mut total: usize = 0;
                loop {
                    match stream.read(&mut buffer) {
                        // A read error ends this stream exactly as its end does.
                        Ok(0) | Err(_) => break,
                        Ok(read) => {
                            total = total.saturating_add(read);
                            if total > limit {
                                flag.store(true, Ordering::SeqCst);
                            }
                            let room = limit.saturating_sub(kept.len());
                            kept.extend_from_slice(&buffer[..read.min(room)]);
                        }
                    }
                }
                kept
            });
            Self {
                handle: Some(handle),
                overflowed,
            }
        }

        fn overflowed(&self) -> bool {
            self.overflowed.load(Ordering::SeqCst)
        }

        fn finished(&self) -> bool {
            self.handle
                .as_ref()
                .is_none_or(std::thread::JoinHandle::is_finished)
        }

        fn take(&mut self) -> Vec<u8> {
            self.handle
                .take()
                .and_then(|handle| handle.join().ok())
                .unwrap_or_default()
        }
    }

    /// Await the child and its drains within the bounds. Returns how the
    /// child exited and the retained stdout and stderr; a bound that ended
    /// the attempt returns its code instead, with the group terminated and
    /// the child reaped.
    fn collect(
        mut child: Child,
        limits: super::Limits,
        cancel: Option<&CancelFlag>,
    ) -> Result<(super::Exited, Vec<u8>, super::Stderr), PrRefreshError> {
        // The leader's pid is also its process group's ID. Nothing below
        // reaps it — the poll observes its exit without consuming it — so
        // this ID stays reserved until the single reap at the end.
        let pid = child.id();
        let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
        let (Some(stdout), Some(stderr)) = (stdout, stderr) else {
            terminate(pid, &mut child);
            return Err(PrRefreshError::ExecutionFailed);
        };
        let mut stdout = Drain::spawn(stdout, limits.max_stdout_bytes);
        let mut stderr = Drain::spawn(stderr, limits.max_stderr_bytes);
        let deadline = Instant::now() + limits.deadline;
        let mut exited = false;
        let mut ended: Option<PrRefreshError> = None;
        loop {
            if !exited {
                match exit_observed(pid) {
                    Ok(seen) => exited = seen,
                    Err(_) => {
                        ended = Some(PrRefreshError::ExecutionFailed);
                        break;
                    }
                }
            }
            // A response past the bound is truncated, so it is unusable even
            // if the rest of the attempt completed.
            if stdout.overflowed() {
                ended = Some(PrRefreshError::OutputTooLarge);
                break;
            }
            // Finished means exited *and* drained: a child that exited while
            // a descendant still holds the pipes is not done, and its exit
            // has not been consumed, so the group can still be terminated.
            if exited && stdout.finished() && stderr.finished() {
                break;
            }
            if cancel.is_some_and(CancelFlag::is_cancelled) {
                ended = Some(PrRefreshError::Cancelled);
                break;
            }
            if Instant::now() >= deadline {
                ended = Some(PrRefreshError::Timeout);
                break;
            }
            std::thread::sleep(POLL);
        }
        if let Some(error) = ended {
            // The whole group goes, leader exited or not, and only then is
            // the leader reaped; a killed group's readers reach the end of
            // their pipes, and are abandoned only if they do not.
            terminate(pid, &mut child);
            abandon_after(DRAIN_GRACE, [stdout, stderr]);
            return Err(error);
        }
        // Nothing is left to signal: the child exited and every writer has
        // closed its pipe. This is the one reap of the whole attempt, and
        // what it returns is the exit status the response is judged by.
        let exited = match child.wait() {
            Ok(status) if status.success() => super::Exited::Success,
            Ok(status) if status.code() == Some(super::GH_EXIT_AUTH) => {
                super::Exited::NeedsAuthentication
            }
            Ok(status) if status.code() == Some(super::GH_EXIT_ERROR) => super::Exited::Error,
            _ => super::Exited::Failed,
        };
        let output = stdout.take();
        let diagnostic = super::Stderr {
            overflowed: stderr.overflowed(),
            kept: stderr.take(),
        };
        Ok((exited, output, diagnostic))
    }

    /// Wait out the drains of a terminated group, then let them go. Only a
    /// descendant that left the group on its own can still hold a pipe here;
    /// its reader keeps a bounded buffer and ends when that pipe closes.
    fn abandon_after(grace: Duration, drains: [Drain; 2]) {
        let deadline = Instant::now() + grace;
        while Instant::now() < deadline && !drains.iter().all(Drain::finished) {
            std::thread::sleep(POLL);
        }
        for mut drain in drains {
            if drain.finished() {
                drop(drain.take());
            }
        }
    }
}
