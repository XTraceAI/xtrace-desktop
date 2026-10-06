//! The bounded GitHub CLI pull-request view.
//!
#![cfg(unix)]
//!
//! Every case runs a synthetic fixture executable written by the test into a
//! temporary directory and synthetic JSON. No network call, no real `gh`, no
//! authentication, no credential is read, and no repository is inspected.

use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command},
    time::{Duration, Instant},
};
use tempfile::TempDir;
use xt_probes::gh::{
    AttemptClock, CancelFlag, DEFAULT_DEADLINE, DEFAULT_MAX_STDERR_BYTES, DEFAULT_MAX_STDOUT_BYTES,
    GhClient, GhClientError, JSON_FIELDS, Limits,
};
use xt_store::{
    SessionMeta, SessionSource, Store,
    pr_link::{
        MAX_ATTEMPTED_AT, MAX_HEAD_REF_LEN, MAX_TITLE_LEN, PrConfidence, PrIdentity,
        PrLinkObservation, PrRefreshError, PrRefreshStatus, PrState, RefreshOutcome, RefreshWrite,
    },
};

const URL: &str = "https://github.com/octo-org/hello.world/pull/42";
const AT: i64 = 1_789_000_000_000;

struct Fixed(i64);

impl AttemptClock for Fixed {
    fn attempted_at(&self) -> i64 {
        self.0
    }
}

fn identity() -> PrIdentity {
    PrIdentity::from_url(URL).unwrap()
}

/// One synthetic executable. The body is plain `sh`; the client itself never
/// uses a shell, so the fixture is the whole program it runs.
fn fixture(directory: &Path, name: &str, body: &str) -> PathBuf {
    let path = directory.join(name);
    fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A fixture that prints one response on stdout and exits 0.
fn responding(directory: &Path, name: &str, json: &str) -> PathBuf {
    fixture(
        directory,
        name,
        &format!("cat <<'XTRACE_FIXTURE_EOF'\n{json}\nXTRACE_FIXTURE_EOF"),
    )
}

fn response(state: &str, merged_at: &str) -> String {
    format!(
        r#"{{"number":42,"title":"Bounded gh client","url":"{URL}","state":"{state}",
            "mergedAt":{merged_at},"additions":12,"deletions":3,
            "headRefName":"feat/pr-refresh-client"}}"#
    )
}

fn short(deadline_ms: u64) -> Limits {
    Limits {
        deadline: Duration::from_millis(deadline_ms),
        ..Limits::default()
    }
}

/// One attempt with a fixed clock and no cancellation.
fn run(executable: &Path, limits: Limits) -> RefreshOutcome {
    GhClient::with_limits(executable, limits)
        .unwrap()
        .pr_view(&identity(), &Fixed(AT), None)
        .unwrap()
}

fn success(outcome: &RefreshOutcome) -> &xt_store::pr_link::RefreshSuccess {
    match outcome {
        RefreshOutcome::Success(success) => success,
        RefreshOutcome::Failure(failure) => panic!("expected a success, got {:?}", failure.error),
    }
}

fn code(outcome: &RefreshOutcome) -> PrRefreshError {
    match outcome {
        RefreshOutcome::Failure(failure) => {
            // Every failure names the requested identity and the one clock
            // reading, never anything taken from the response.
            assert_eq!(failure.pull_request, identity());
            assert_eq!(failure.attempted_at, AT);
            failure.error
        }
        RefreshOutcome::Success(success) => panic!("expected a failure, got {success:?}"),
    }
}

/// Whether a process is still alive, for the termination cases.
fn alive(pid: i32) -> bool {
    // SAFETY: signal 0 only probes for the process.
    unsafe { libc::kill(pid, 0) == 0 }
}

fn wait_until_gone(pid: i32) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && alive(pid) {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!alive(pid), "process {pid} outlived the attempt");
}

/// The pid a fixture recorded, waiting up to `grace` for the write to land.
/// `None` means the fixture never got that far, which is a real outcome when
/// the attempt itself is what ended the fixture.
fn observed_pid(path: &Path, grace: Duration) -> Option<i32> {
    let deadline = Instant::now() + grace;
    loop {
        if let Ok(text) = fs::read_to_string(path)
            && let Ok(pid) = text.trim().parse()
        {
            return Some(pid);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The pid of a fixture the case waits on before acting: a readiness signal,
/// so the process it names certainly exists by the time anything is asserted
/// about it. A case that lets a deadline end the attempt must use
/// `observed_pid` instead — a fixture killed during its first statement
/// records nothing, and no later read can recover a pid it never wrote.
fn recorded_pid(path: &Path) -> i32 {
    observed_pid(path, Duration::from_secs(10)).expect("the fixture recorded no pid")
}

/// A process outside the client's group, killed and reaped when the case ends.
/// The guard runs on an assertion failure too, so a failing run leaves nothing
/// of its own behind for the next one.
struct Bystander(Child);

impl Bystander {
    fn spawn() -> Self {
        Self(
            Command::new("/bin/sh")
                .args(["-c", "sleep 30"])
                .spawn()
                .unwrap(),
        )
    }
    fn alive(&self) -> bool {
        alive(i32::try_from(self.0.id()).unwrap())
    }
}

impl Drop for Bystander {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Run one attempt on a worker thread, wait until the fixture has recorded its
/// pid at `ready`, then cancel. Returns the outcome, that pid and how long the
/// attempt took.
///
/// The client keeps its own default deadline: generous enough that it cannot
/// be what ends the attempt, still bounded, and the production limits are
/// untouched. Cancelling on observed readiness rather than on a clock is what
/// makes the group cases deterministic — the group being terminated is known
/// to exist, so the assertions below always have a process to describe.
/// Cancellation and a deadline reach the same termination in the client; only
/// the trigger differs, and this one cannot race the fixture's first write.
fn cancel_once_started(executable: &Path, ready: &Path) -> (RefreshOutcome, i32, Duration) {
    let client = GhClient::with_limits(executable, Limits::default()).unwrap();
    let cancel = CancelFlag::new();
    let flag = cancel.clone();
    let started = Instant::now();
    let attempt = std::thread::spawn(move || {
        client
            .pr_view(&identity(), &Fixed(AT), Some(&flag))
            .unwrap()
    });
    let pid = recorded_pid(ready);
    cancel.cancel();
    let outcome = attempt.join().unwrap();
    (outcome, pid, started.elapsed())
}

#[test]
fn open_closed_and_merged_responses_become_typed_successes() {
    let directory = TempDir::new().unwrap();
    let cases = [
        (PrState::Open, "OPEN", "null", None),
        (PrState::Closed, "CLOSED", "null", None),
        (
            PrState::Merged,
            "MERGED",
            "\"2026-09-18T07:21:09Z\"",
            Some("2026-09-18T07:21:09Z"),
        ),
    ];
    for (state, spelling, merged_at, expected) in cases {
        let executable = responding(
            directory.path(),
            &format!("gh-{spelling}"),
            &response(spelling, merged_at),
        );
        let outcome = run(&executable, short(10_000));
        let success = success(&outcome);
        assert_eq!(success.pull_request, identity());
        assert_eq!(success.attempted_at, AT);
        assert_eq!(success.title, "Bounded gh client");
        assert_eq!(success.state, state);
        assert_eq!(success.merged_at.as_deref(), expected);
        assert_eq!(success.additions, 12);
        assert_eq!(success.deletions, 3);
        assert_eq!(success.head_ref_name, "feat/pr-refresh-client");
        // The result is exactly what storage accepts, by storage's own rules.
        outcome.validate().unwrap();
    }
}

#[test]
fn a_parsed_success_is_accepted_by_the_store() {
    let directory = TempDir::new().unwrap();
    let executable = responding(
        directory.path(),
        "gh-store",
        &response("MERGED", "\"2026-09-18T07:21:09Z\""),
    );
    let outcome = run(&executable, short(10_000));

    // The client holds no store, transaction or lock; the caller records the
    // finished result afterwards.
    let mut store = Store::open_in_memory().unwrap();
    store
        .upsert_session(
            &SessionMeta::new("session-a", "claude", SessionSource::Fixture),
            false,
        )
        .unwrap();
    store
        .record_pr_link(&PrLinkObservation {
            session_id: "session-a".into(),
            pull_request: identity(),
            confidence: PrConfidence::Exact,
            first_seen_at: 10,
            last_seen_at: 20,
        })
        .unwrap();
    assert_eq!(
        store.record_pr_refresh(&outcome).unwrap(),
        RefreshWrite::Applied
    );
    let stored = store.pull_request(&identity()).unwrap().unwrap();
    assert_eq!(stored.title.as_deref(), Some("Bounded gh client"));
    assert_eq!(stored.state, Some(PrState::Merged));
    assert_eq!(stored.merged_at.as_deref(), Some("2026-09-18T07:21:09Z"));
    assert_eq!(stored.additions, Some(12));
    assert_eq!(stored.deletions, Some(3));
    assert_eq!(
        stored.head_ref_name.as_deref(),
        Some("feat/pr-refresh-client")
    );
    assert_eq!(stored.refreshed_at, Some(AT));
    assert_eq!(stored.last_attempted_at, Some(AT));
    assert_eq!(stored.refresh_status(), PrRefreshStatus::Refreshed);
    // A replay of the same attempt is the storage layer's decision, not ours.
    assert_eq!(
        store.record_pr_refresh(&outcome).unwrap(),
        RefreshWrite::Unchanged
    );
}

#[test]
fn the_child_sees_the_exact_argv_no_stdin_and_no_interactive_environment() {
    let directory = TempDir::new().unwrap();
    let observed = directory.path().join("observed");
    fs::create_dir(&observed).unwrap();
    let json = response("OPEN", "null");
    let body = format!(
        r#"printf '%s\n' "$0" "$@" > '{observed}/argv'
if IFS= read -r line; then printf 'read:%s' "$line" > '{observed}/stdin'; else printf 'eof' > '{observed}/stdin'; fi
{{
  printf 'GH_PROMPT_DISABLED=%s\n' "${{GH_PROMPT_DISABLED-unset}}"
  printf 'GH_NO_UPDATE_NOTIFIER=%s\n' "${{GH_NO_UPDATE_NOTIFIER-unset}}"
  printf 'GH_NO_EXTENSION_UPDATE_NOTIFIER=%s\n' "${{GH_NO_EXTENSION_UPDATE_NOTIFIER-unset}}"
  printf 'GH_SPINNER_DISABLED=%s\n' "${{GH_SPINNER_DISABLED-unset}}"
  printf 'GH_PAGER=[%s]\n' "${{GH_PAGER-unset}}"
  printf 'PAGER=[%s]\n' "${{PAGER-unset}}"
  printf 'NO_COLOR=%s\n' "${{NO_COLOR-unset}}"
  printf 'CLICOLOR=%s\n' "${{CLICOLOR-unset}}"
  printf 'GH_FORCE_TTY=%s\n' "${{GH_FORCE_TTY-unset}}"
  printf 'GH_DEBUG=%s\n' "${{GH_DEBUG-unset}}"
  printf 'GH_REPO=%s\n' "${{GH_REPO-unset}}"
  printf 'PATH=%s\n' "${{PATH-unset}}"
  printf 'pwd=%s\n' "$(pwd)"
}} > '{observed}/environment'
cat <<'XTRACE_FIXTURE_EOF'
{json}
XTRACE_FIXTURE_EOF"#,
        observed = observed.display(),
    );
    let executable = fixture(directory.path(), "gh-observe", &body);
    let client = GhClient::with_limits(&executable, short(10_000)).unwrap();
    let outcome = client.pr_view(&identity(), &Fixed(AT), None).unwrap();
    success(&outcome);

    let argv: Vec<String> = fs::read_to_string(observed.join("argv"))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    assert_eq!(
        argv,
        vec![
            executable.display().to_string(),
            "pr".to_owned(),
            "view".to_owned(),
            URL.to_owned(),
            "--json".to_owned(),
            JSON_FIELDS.to_owned(),
        ]
    );
    assert_eq!(
        argv,
        client
            .argv(&identity())
            .iter()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
    );
    // Nothing is ever read from the child's standard input.
    assert_eq!(fs::read_to_string(observed.join("stdin")).unwrap(), "eof");

    let environment = fs::read_to_string(observed.join("environment")).unwrap();
    let lines: Vec<&str> = environment.lines().collect();
    assert_eq!(
        lines[..11],
        [
            "GH_PROMPT_DISABLED=1",
            "GH_NO_UPDATE_NOTIFIER=1",
            "GH_NO_EXTENSION_UPDATE_NOTIFIER=1",
            "GH_SPINNER_DISABLED=1",
            "GH_PAGER=[]",
            "PAGER=[]",
            "NO_COLOR=1",
            "CLICOLOR=0",
            "GH_FORCE_TTY=unset",
            "GH_DEBUG=unset",
            "GH_REPO=unset",
        ]
    );
    // The rest of the environment, credentials included, is inherited as it
    // stands: the client sets what must not be interactive and reads nothing.
    assert_eq!(
        lines[11],
        format!("PATH={}", std::env::var("PATH").unwrap())
    );
    // No repository can be inferred from the caller's working directory.
    assert_eq!(lines[12], "pwd=/");
}

#[test]
fn a_response_for_another_pull_request_is_rejected() {
    let directory = TempDir::new().unwrap();
    let cases = [
        // Another number, in both the URL and the number field.
        (
            "other-number",
            r#"{"number":43,"title":"t","url":"https://github.com/octo-org/hello.world/pull/43","state":"OPEN","mergedAt":null,"additions":1,"deletions":0,"headRefName":"b"}"#,
        ),
        // Another repository.
        (
            "other-repo",
            r#"{"number":42,"title":"t","url":"https://github.com/octo-org/other/pull/42","state":"OPEN","mergedAt":null,"additions":1,"deletions":0,"headRefName":"b"}"#,
        ),
        // A URL and a number that disagree with each other.
        (
            "split",
            r#"{"number":7,"title":"t","url":"https://github.com/octo-org/hello.world/pull/42","state":"OPEN","mergedAt":null,"additions":1,"deletions":0,"headRefName":"b"}"#,
        ),
        // A non-canonical spelling of the requested identity.
        (
            "alias",
            r#"{"number":42,"title":"t","url":"https://github.com/octo-org/hello.world/pull/42/","state":"OPEN","mergedAt":null,"additions":1,"deletions":0,"headRefName":"b"}"#,
        ),
        // Another host entirely.
        (
            "other-host",
            r#"{"number":42,"title":"t","url":"https://example.com/octo-org/hello.world/pull/42","state":"OPEN","mergedAt":null,"additions":1,"deletions":0,"headRefName":"b"}"#,
        ),
    ];
    for (name, json) in cases {
        let executable = responding(directory.path(), name, json);
        assert_eq!(
            code(&run(&executable, short(10_000))),
            PrRefreshError::InvalidResponse,
            "{name} should not be accepted",
        );
    }
    // An ASCII case variant of the same identity is the same pull request.
    let executable = responding(
        directory.path(),
        "case-variant",
        &response("OPEN", "null").replace(URL, "https://GitHub.com/Octo-Org/Hello.World/pull/42"),
    );
    assert_eq!(success(&run(&executable, short(10_000))).additions, 12);
}

#[test]
fn malformed_empty_and_wrongly_typed_responses_are_invalid() {
    let directory = TempDir::new().unwrap();
    let long_title = "t".repeat(MAX_TITLE_LEN + 1);
    let long_ref = "b".repeat(MAX_HEAD_REF_LEN + 1);
    let cases: Vec<(&str, String)> = vec![
        ("empty", String::new()),
        ("blank", "   \n".to_owned()),
        ("not-json", "gh: something happened".to_owned()),
        ("array", format!("[{}]", response("OPEN", "null"))),
        (
            "trailing-object",
            format!("{}{}", response("OPEN", "null"), response("OPEN", "null")),
        ),
        (
            "missing-field",
            r#"{"number":42,"title":"t","url":"https://github.com/octo-org/hello.world/pull/42","state":"OPEN","mergedAt":null,"additions":1,"headRefName":"b"}"#.to_owned(),
        ),
        (
            "unknown-field",
            response("OPEN", "null").replace(r#""additions":12"#, r#""body":"x","additions":12"#),
        ),
        (
            "number-as-string",
            response("OPEN", "null").replace(r#""number":42"#, r#""number":"42""#),
        ),
        (
            "additions-as-float",
            response("OPEN", "null").replace(r#""additions":12"#, r#""additions":12.5"#),
        ),
        (
            "additions-as-string",
            response("OPEN", "null").replace(r#""additions":12"#, r#""additions":"12""#),
        ),
        (
            "negative-additions",
            response("OPEN", "null").replace(r#""additions":12"#, r#""additions":-1"#),
        ),
        (
            "negative-deletions",
            response("OPEN", "null").replace(r#""deletions":3"#, r#""deletions":-3"#),
        ),
        (
            "unknown-state",
            response("DRAFT", "null"),
        ),
        (
            "lowercase-state",
            response("open", "null"),
        ),
        (
            "merged-without-time",
            response("MERGED", "null"),
        ),
        (
            "merged-with-bad-time",
            response("MERGED", "\"18 September 2026\""),
        ),
        (
            "merged-with-number",
            response("MERGED", "1789000000000"),
        ),
        (
            "open-with-merge-time",
            response("OPEN", "\"2026-09-18T07:21:09Z\""),
        ),
        (
            "blank-title",
            response("OPEN", "null").replace("Bounded gh client", "   "),
        ),
        (
            "control-in-title",
            response("OPEN", "null").replace("Bounded gh client", "bounded\\u0007client"),
        ),
        (
            "control-in-head-ref",
            response("OPEN", "null").replace("feat/pr-refresh-client", "feat\\u0000client"),
        ),
        (
            "long-title",
            response("OPEN", "null").replace("Bounded gh client", &long_title),
        ),
        (
            "long-head-ref",
            response("OPEN", "null").replace("feat/pr-refresh-client", &long_ref),
        ),
        (
            "blank-head-ref",
            response("OPEN", "null").replace("feat/pr-refresh-client", " "),
        ),
    ];
    for (name, json) in cases {
        let executable = responding(directory.path(), name, &json);
        assert_eq!(
            code(&run(&executable, short(10_000))),
            PrRefreshError::InvalidResponse,
            "{name} should not be accepted",
        );
    }
    // Invalid UTF-8 is not a response either.
    let executable = fixture(
        directory.path(),
        "not-utf8",
        "printf '{\"number\":42,\"title\":\"\\376\\377\"}'",
    );
    assert_eq!(
        code(&run(&executable, short(10_000))),
        PrRefreshError::InvalidResponse
    );
}

#[test]
fn the_merged_at_key_must_be_present_even_when_null() {
    let directory = TempDir::new().unwrap();
    // A response that simply omits the key is not the documented response,
    // whatever the state: the field was requested, so its absence means the
    // answer is not the one that was asked for.
    for state in ["OPEN", "CLOSED", "MERGED"] {
        let json = response(state, "null").replace("\"mergedAt\":null,", "");
        assert!(!json.contains("mergedAt"));
        let executable = responding(directory.path(), &format!("absent-{state}"), &json);
        assert_eq!(
            code(&run(&executable, short(10_000))),
            PrRefreshError::InvalidResponse,
            "{state} without a mergedAt key should not be accepted",
        );
    }
    // An explicit null is the documented "not merged" and is accepted, for
    // the two states that may carry it.
    for state in ["OPEN", "CLOSED"] {
        let executable = responding(
            directory.path(),
            &format!("null-{state}"),
            &response(state, "null"),
        );
        assert_eq!(success(&run(&executable, short(10_000))).merged_at, None);
    }
    // MERGED still needs a real RFC3339 instant, and still refuses null.
    let executable = responding(
        directory.path(),
        "merged-with-time",
        &response("MERGED", "\"2026-09-18T07:21:09Z\""),
    );
    assert_eq!(
        success(&run(&executable, short(10_000)))
            .merged_at
            .as_deref(),
        Some("2026-09-18T07:21:09Z")
    );
    let executable = responding(directory.path(), "merged-null", &response("MERGED", "null"));
    assert_eq!(
        code(&run(&executable, short(10_000))),
        PrRefreshError::InvalidResponse
    );
}

#[test]
fn an_oversized_response_ends_the_attempt() {
    let directory = TempDir::new().unwrap();
    let pid = directory.path().join("pid");
    let executable = fixture(
        directory.path(),
        "gh-flood",
        &format!(
            "printf '%s' \"$$\" > '{pid}'\nhead -c 400000 /dev/zero | tr '\\0' 'x'\nsleep 30",
            pid = pid.display()
        ),
    );
    let limits = Limits {
        deadline: Duration::from_secs(10),
        max_stdout_bytes: 4096,
        ..Limits::default()
    };
    let started = Instant::now();
    assert_eq!(
        code(&run(&executable, limits)),
        PrRefreshError::OutputTooLarge
    );
    // The bound ends the attempt; it does not wait for the deadline.
    assert!(started.elapsed() < Duration::from_secs(9));
    wait_until_gone(recorded_pid(&pid));

    // The documented bounds are the defaults.
    assert_eq!(DEFAULT_MAX_STDOUT_BYTES, 1024 * 1024);
    assert_eq!(DEFAULT_MAX_STDERR_BYTES, 64 * 1024);
    assert_eq!(DEFAULT_DEADLINE, Duration::from_secs(30));
    assert_eq!(
        GhClient::new("/usr/bin/true").unwrap().limits(),
        Limits::default()
    );
}

#[test]
fn stderr_pressure_neither_blocks_the_child_nor_reaches_the_result() {
    let directory = TempDir::new().unwrap();
    let json = response("OPEN", "null");
    // Far more stderr than any pipe buffer holds, written before the
    // response: a client that did not drain it concurrently would deadlock.
    let executable = fixture(
        directory.path(),
        "gh-noisy",
        &format!(
            "i=0\nwhile [ $i -lt 2000 ]; do printf 'SYNTHETIC-STDERR-MARKER %04d %s\\n' \"$i\" \
             'xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx' \
             1>&2; i=$((i+1)); done\ncat <<'XTRACE_FIXTURE_EOF'\n{json}\nXTRACE_FIXTURE_EOF"
        ),
    );
    let limits = Limits {
        deadline: Duration::from_secs(10),
        max_stderr_bytes: 512,
        ..Limits::default()
    };
    let outcome = run(&executable, limits);
    assert_eq!(success(&outcome).title, "Bounded gh client");
    // Nothing the child wrote to stderr is carried anywhere.
    assert!(!format!("{outcome:?}").contains("SYNTHETIC-STDERR-MARKER"));
}

#[test]
fn a_process_that_ran_and_failed_is_one_execution_failure() {
    let directory = TempDir::new().unwrap();
    // Stderr that names a cause is still not evidence of that cause here.
    let cases = [
        (
            "exit-1",
            "printf 'gh: Could not resolve to a PullRequest\\n' 1>&2\nexit 1",
        ),
        (
            "exit-2",
            "printf 'gh: authentication required\\n' 1>&2\nexit 2",
        ),
        (
            "rate-limited",
            "printf 'API rate limit exceeded\\n' 1>&2\nexit 1",
        ),
        (
            "not-found-on-stdout",
            "printf 'HTTP 404: Not Found\\n'\nexit 1",
        ),
        ("signal", "kill -TERM $$\nsleep 5"),
        (
            "valid-json-nonzero-exit",
            "printf '%s' '{\"number\":42,\"title\":\"t\",\"url\":\"https://github.com/octo-org/hello.world/pull/42\",\"state\":\"OPEN\",\"mergedAt\":null,\"additions\":1,\"deletions\":0,\"headRefName\":\"b\"}'\nexit 1",
        ),
    ];
    for (name, body) in cases {
        let executable = fixture(directory.path(), name, body);
        let outcome = run(&executable, short(10_000));
        assert_eq!(
            code(&outcome),
            PrRefreshError::ExecutionFailed,
            "{name} should be one execution failure",
        );
        let reported = format!("{outcome:?}");
        for leak in ["404", "authentication", "rate limit", "PullRequest"] {
            assert!(!reported.contains(leak), "{name} leaked {leak}");
        }
    }
}

/// gh documents exit code 4 as "authentication required" (`gh help
/// exit-codes`); that exit status, and nothing it writes, is unauthorized.
/// Any other nonzero exit stays one execution failure, whatever it says.
#[test]
fn exit_code_four_is_unauthorized_and_any_other_failure_is_not() {
    let directory = TempDir::new().unwrap();
    let signed_out = fixture(
        directory.path(),
        "gh-signed-out",
        "printf 'To get started with GitHub CLI, please run:  gh auth login\\n' 1>&2\nexit 4",
    );
    let outcome = run(&signed_out, short(10_000));
    assert_eq!(code(&outcome), PrRefreshError::Unauthorized);
    assert!(!format!("{outcome:?}").contains("gh auth login"));
    // Exit 4 decides it, not the words: silence still is unauthorized.
    let silent = fixture(directory.path(), "gh-exit-4-silent", "exit 4");
    assert_eq!(
        code(&run(&silent, short(10_000))),
        PrRefreshError::Unauthorized
    );
    // The same words with another exit code are not.
    let other = fixture(
        directory.path(),
        "gh-exit-1-auth-words",
        "printf 'please run: gh auth login\\n' 1>&2\nexit 1",
    );
    assert_eq!(
        code(&run(&other, short(10_000))),
        PrRefreshError::ExecutionFailed
    );
}

#[test]
fn an_executable_that_cannot_run_is_unavailable() {
    let directory = TempDir::new().unwrap();
    let missing = directory.path().join("absent");
    assert_eq!(
        code(&run(&missing, short(10_000))),
        PrRefreshError::Unavailable
    );
    let unreadable = directory.path().join("not-executable");
    fs::write(&unreadable, "#!/bin/sh\necho hi\n").unwrap();
    fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        code(&run(&unreadable, short(10_000))),
        PrRefreshError::Unavailable
    );
    let as_directory = directory.path().to_path_buf();
    assert_eq!(
        code(&run(&as_directory, short(10_000))),
        PrRefreshError::Unavailable
    );
}

#[test]
fn a_cancel_before_the_run_starts_nothing() {
    let directory = TempDir::new().unwrap();
    let marker = directory.path().join("ran");
    let executable = fixture(
        directory.path(),
        "gh-never",
        &format!("printf 'ran' > '{}'", marker.display()),
    );
    let cancel = CancelFlag::new();
    cancel.cancel();
    let outcome = GhClient::with_limits(&executable, short(10_000))
        .unwrap()
        .pr_view(&identity(), &Fixed(AT), Some(&cancel))
        .unwrap();
    assert_eq!(code(&outcome), PrRefreshError::Cancelled);
    assert!(!marker.exists(), "a cancelled attempt started a process");
}

#[test]
fn a_cancel_during_the_run_kills_and_reaps_the_child() {
    let directory = TempDir::new().unwrap();
    let pid = directory.path().join("pid");
    let executable = fixture(
        directory.path(),
        "gh-sleep",
        &format!("printf '%s' \"$$\" > '{}'\nsleep 60", pid.display()),
    );
    let cancel = CancelFlag::new();
    let flag = cancel.clone();
    let watched = pid.clone();
    let canceller = std::thread::spawn(move || {
        recorded_pid(&watched);
        flag.cancel();
    });
    let started = Instant::now();
    let outcome = GhClient::with_limits(&executable, Limits::default())
        .unwrap()
        .pr_view(&identity(), &Fixed(AT), Some(&cancel))
        .unwrap();
    canceller.join().unwrap();
    assert_eq!(code(&outcome), PrRefreshError::Cancelled);
    // The cancel ends the attempt long before the 30 second deadline.
    assert!(started.elapsed() < Duration::from_secs(10));
    wait_until_gone(recorded_pid(&pid));
    assert!(!zombies(recorded_pid(&pid)));
}

#[test]
fn a_run_past_the_deadline_times_out() {
    let directory = TempDir::new().unwrap();
    let pid = directory.path().join("pid");
    let executable = fixture(
        directory.path(),
        "gh-slow",
        &format!("printf '%s' \"$$\" > '{}'\nsleep 60", pid.display()),
    );
    // Long enough that the fixture certainly starts under load, short
    // enough that the deadline, not the sleep, ends the attempt.
    let started = Instant::now();
    assert_eq!(
        code(&run(&executable, short(2_000))),
        PrRefreshError::Timeout
    );
    assert!((Duration::from_secs(2)..Duration::from_secs(20)).contains(&started.elapsed()));
    // The deadline is this case's whole subject, and it is allowed to arrive
    // before the fixture recorded anything: the timeout contract above holds
    // either way. When there is a pid to follow, the process it names must
    // still be gone and reaped. The group cases cancel on observed readiness
    // instead, so that evidence never rests on winning this race. The group
    // is dead by now, so a pid that has not appeared is one that never will.
    if let Some(child) = observed_pid(&pid, Duration::from_millis(500)) {
        wait_until_gone(child);
        assert!(!zombies(child));
    }
}

#[test]
fn a_descendant_holding_the_pipes_is_killed_with_the_group() {
    let directory = TempDir::new().unwrap();
    let pid = directory.path().join("descendant");
    // The launcher exits at once, leaving a descendant that inherited both
    // pipes. The attempt is not finished until those pipes close.
    let executable = fixture(
        directory.path(),
        "gh-launcher",
        &format!(
            "sh -c 'printf \"%s\" \"$$\" > \"{pid}\"; sleep 60' &\nexit 0",
            pid = pid.display()
        ),
    );
    let (outcome, descendant, elapsed) = cancel_once_started(&executable, &pid);
    // The launcher exited long before this; that the attempt was still open to
    // be cancelled is the descendant holding both pipes. Ending it terminates
    // the whole group the launcher left behind, deadline or cancel alike.
    assert_eq!(code(&outcome), PrRefreshError::Cancelled);
    assert!(elapsed < Duration::from_secs(20));
    wait_until_gone(descendant);
    assert!(!zombies(descendant));
}

#[test]
fn the_leader_stays_unreaped_until_its_drains_end() {
    let directory = TempDir::new().unwrap();
    let leader_path = directory.path().join("leader");
    let descendant_path = directory.path().join("descendant");
    let json = directory.path().join("response.json");
    fs::write(&json, response("OPEN", "null")).unwrap();
    // The launcher exits immediately; a descendant keeps both pipes and only
    // answers a second later. Until that answer has been drained the leader
    // must stay unreaped, because its pid is what reserves the ID of the
    // process group a termination would have to signal.
    let executable = fixture(
        directory.path(),
        "gh-early-exit",
        &format!(
            "printf '%s' \"$$\" > '{leader}'\n\
             sh -c 'printf \"%s\" \"$$\" > \"{descendant}\"; sleep 1; cat \"{json}\"' &\n\
             exit 0",
            leader = leader_path.display(),
            descendant = descendant_path.display(),
            json = json.display(),
        ),
    );
    let client = GhClient::with_limits(&executable, short(20_000)).unwrap();
    let attempt =
        std::thread::spawn(move || client.pr_view(&identity(), &Fixed(AT), None).unwrap());
    let leader = recorded_pid(&leader_path);
    let descendant = recorded_pid(&descendant_path);
    // The launcher has exited by now, but its pipes have not closed.
    std::thread::sleep(Duration::from_millis(400));
    assert!(
        alive(leader),
        "the leader was reaped while its drains were still running",
    );

    // The descendant's answer is still the attempt's answer, and the leader
    // is reaped exactly once, after the drains ended.
    let outcome = attempt.join().unwrap();
    assert_eq!(success(&outcome).title, "Bounded gh client");
    wait_until_gone(leader);
    wait_until_gone(descendant);
    assert!(!zombies(leader));
}

#[test]
fn terminating_a_group_leaves_unrelated_processes_alone() {
    let directory = TempDir::new().unwrap();
    let pid = directory.path().join("pid");
    // A bystander in this test's own process group, not the client's.
    let bystander = Bystander::spawn();
    let executable = fixture(
        directory.path(),
        "gh-grouped",
        &format!(
            "sh -c 'printf \"%s\" \"$$\" > \"{pid}\"; sleep 60' &\nexit 0",
            pid = pid.display()
        ),
    );
    let (outcome, descendant, _) = cancel_once_started(&executable, &pid);
    assert_eq!(code(&outcome), PrRefreshError::Cancelled);
    wait_until_gone(descendant);
    // Only the client's own group was signalled.
    assert!(
        bystander.alive(),
        "a process outside the attempt's group was signalled",
    );
}

/// Whether the process is a zombie the client failed to reap.
fn zombies(pid: i32) -> bool {
    let output = Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .starts_with('Z')
}

#[test]
fn only_canonical_identities_reach_the_argument_vector() {
    // Identities the reviewed parser refuses can never become an argument.
    for rejected in [
        "https://github.com/octo-org/hello.world/pull/42/",
        "https://github.com/octo-org/hello.world.git/pull/42",
        "https://github.com/octo-org/hello.world/pull/042",
        "https://github.com/octo-org/hello.world/pull/4 2",
        "https://github.com/octo-org/hello.world/pull/42?x=1",
        "https://github.com/octo-org/hello.world/pull/42#top",
        "https://www.github.com/octo-org/hello.world/pull/42",
        "http://github.com/octo-org/hello.world/pull/42",
        "https://github.com:443/octo-org/hello.world/pull/42",
        "https://user@github.com/octo-org/hello.world/pull/42",
        "https://github.com/octo-org/hello.world/pull/42/files",
        "https://github.com/octo-org/hello%2Eworld/pull/42",
        "https://github.com/octo-org/../hello.world/pull/42",
        "https://github.com/octo-org/hello.world/pull/-1",
        "https://github.com/octo-org/hello.world/pulls/42",
        "https://github.com/octo-org/hello.world/pull/42 --json body",
    ] {
        assert!(
            PrIdentity::from_url(rejected).is_err(),
            "{rejected} must not parse"
        );
    }
    // An accepted but oddly spelled identity is argued canonically.
    let client = GhClient::new("/usr/bin/true").unwrap();
    let odd = PrIdentity::from_url("https://GITHUB.com/Octo-Org/Hello.World/pull/9007199254740993")
        .unwrap();
    let argv: Vec<String> = client
        .argv(&odd)
        .iter()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        argv,
        [
            "/usr/bin/true",
            "pr",
            "view",
            "https://github.com/octo-org/hello.world/pull/9007199254740993",
            "--json",
            JSON_FIELDS,
        ]
    );
    assert_eq!(argv.len(), 6);
    assert_eq!(
        JSON_FIELDS,
        "number,title,url,state,mergedAt,additions,deletions,headRefName"
    );
}

#[test]
fn a_client_is_refused_rather_than_guessing() {
    assert_eq!(
        GhClient::new("gh").unwrap_err(),
        GhClientError::ExecutableNotAbsolute
    );
    assert_eq!(
        GhClient::new("./gh").unwrap_err(),
        GhClientError::ExecutableNotAbsolute
    );
    assert_eq!(
        GhClient::new("").unwrap_err(),
        GhClientError::ExecutableNotAbsolute
    );
    for limits in [
        Limits {
            deadline: Duration::ZERO,
            ..Limits::default()
        },
        Limits {
            max_stdout_bytes: 0,
            ..Limits::default()
        },
        Limits {
            max_stderr_bytes: 0,
            ..Limits::default()
        },
    ] {
        assert_eq!(
            GhClient::with_limits("/usr/bin/true", limits).unwrap_err(),
            GhClientError::InvalidLimits
        );
    }
    // An attempt time storage could not hold is refused before anything runs.
    let client = GhClient::new("/usr/bin/true").unwrap();
    for reading in [-1, MAX_ATTEMPTED_AT + 1, i64::MIN, i64::MAX] {
        assert_eq!(
            client
                .pr_view(&identity(), &Fixed(reading), None)
                .unwrap_err(),
            GhClientError::AttemptTimeOutOfRange
        );
    }
    // Unsupported platforms are reported, never attempted.
    assert_eq!(cfg!(unix), GhClient::new("/usr/bin/true").is_ok());
}

#[test]
fn the_client_never_inspects_or_clears_the_credential_environment() {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("gh.rs"),
    )
    .unwrap();
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for forbidden in [
        // Never read the parent's environment, so no credential can be
        // inspected, logged or stored.
        "env::var",
        "env::vars",
        "std::env",
        // Never replace the child's environment, so credentials still reach
        // the GitHub CLI.
        "env_clear",
        ".envs(",
        // Never a shell, and never the caller's standard input or terminal.
        "sh -c",
        "Stdio::inherit",
    ] {
        assert!(
            !code.contains(forbidden),
            "the client source must not use {forbidden}"
        );
    }
    // The only two credential-bearing names in the file are in prose.
    for name in ["GH_TOKEN", "GITHUB_TOKEN"] {
        assert!(!code.contains(name), "{name} must not appear in code");
    }
}

#[test]
fn the_client_holds_no_store_while_a_process_runs() {
    // A running attempt owns nothing but its child: the store is opened and
    // written by the caller, before or after, never across the process work.
    let directory = TempDir::new().unwrap();
    let executable = responding(directory.path(), "gh-open", &response("OPEN", "null"));
    let client = GhClient::with_limits(&executable, short(10_000)).unwrap();
    let mut store = Store::open_in_memory().unwrap();
    store
        .upsert_session(
            &SessionMeta::new("session-a", "claude", SessionSource::Fixture),
            false,
        )
        .unwrap();
    store
        .record_pr_link(&PrLinkObservation {
            session_id: "session-a".into(),
            pull_request: identity(),
            confidence: PrConfidence::Exact,
            first_seen_at: 1,
            last_seen_at: 2,
        })
        .unwrap();
    // Two attempts run with no transaction open; only their results are then
    // handed to the store, newest last.
    let first = client.pr_view(&identity(), &Fixed(AT), None).unwrap();
    let second = client.pr_view(&identity(), &Fixed(AT + 1), None).unwrap();
    assert_eq!(
        store.record_pr_refresh(&first).unwrap(),
        RefreshWrite::Applied
    );
    assert_eq!(
        store.record_pr_refresh(&second).unwrap(),
        RefreshWrite::Applied
    );
    assert_eq!(
        store.record_pr_refresh(&first).unwrap(),
        RefreshWrite::Stale
    );
}
