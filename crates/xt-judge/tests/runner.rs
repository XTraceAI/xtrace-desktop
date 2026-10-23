//! Grading runs against a fake `claude`.
#![cfg(unix)]
//!
//! Every case writes a synthetic `claude` (a small Python program) into a
//! fresh folder under Cargo's target temp folder, with a synthetic project,
//! home folder and grade folder beside it. The real Claude Code CLI is never
//! run; the real home folder and XTrace's data folders are never read: the
//! inherited environment given to the runner is synthetic too.

use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use xt_judge::{
    CancelFlag, GradeError, GradeRun, GradingSkill, Limits, ResultError, Severity, runner, skill,
};

fn vendor() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendor"))
}

fn bundled() -> GradingSkill {
    GradingSkill::open_builtin(
        &skill::bundled_dir(&vendor(), "claude-md-doctor").unwrap(),
        "claude-md-doctor",
    )
    .unwrap()
}

/// The host's python3, for the fake's `#!` line. Tests need it anyway: the
/// bundled skill is Python.
fn python3() -> PathBuf {
    xt_probes::executable::resolve_within(
        "python3",
        None,
        std::env::var_os("PATH").as_deref(),
        &[Path::new("/usr/bin"), Path::new("/opt/homebrew/bin")],
    )
    .expect("python3 is required for these tests")
}

/// What Claude Code 2.1.296 prints for `--version`.
const CURRENT_VERSION: &str = "    print('2.1.296 (Claude Code)')";

const GOOD: &str = r#"{"protocol": 1, "score": 75, "summary": "One dead reference.",
  "insights": [{"severity": "critical", "title": "Dead path", "fix": "Remove it."},
               {"severity": "warning", "title": "Long"}],
  "criteria": [{"name": "Health of the memory files", "max": 100, "awarded": 75}],
  "report": "report.html", "complete": true}"#;

struct Case {
    _root: tempfile::TempDir,
    root: PathBuf,
    project: PathBuf,
    grade: PathBuf,
    home: PathBuf,
    claude: PathBuf,
    record: PathBuf,
    pids: PathBuf,
}

impl Case {
    /// A fake whose behaviour is `body`, Python run after it has recorded its
    /// argv, environment and working folder. `G` is the grade folder from
    /// its environment. Asked `--version`, it answers as Claude Code 2.1.296
    /// does and records nothing.
    fn new(body: &str) -> Self {
        Self::with_version(CURRENT_VERSION, body)
    }

    /// A fake whose answer to `--version` is the indented Python `version`.
    fn with_version(version: &str, body: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("grade-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let (project, grade, home, bin) = (
            root.join("project"),
            root.join("grade"),
            root.join("home"),
            root.join("bin"),
        );
        for folder in [&project, &grade, &home, &bin] {
            fs::create_dir_all(folder).unwrap();
        }
        let record = root.join("record.json");
        let pids = root.join("pids");
        let claude = bin.join("claude");
        let script = format!(
            "#!{python} -I\nimport json, os, subprocess, sys, time\n\
             RECORD = {record:?}\nPIDS = {pids:?}\n\
             if sys.argv[1:] == ['--version']:\n{version}\n    sys.exit(0)\n\
             with open(RECORD, 'w') as f:\n    json.dump({{'argv': sys.argv, 'env': dict(os.environ), 'cwd': os.getcwd()}}, f)\n\
             G = os.environ['XTRACE_GRADE_DIR']\n\
             def write(name, text):\n    with open(os.path.join(G, name), 'w') as f:\n        f.write(text)\n\
             GOOD = {good:?}\n{body}\n",
            python = python3().display(),
            record = record.to_str().unwrap(),
            pids = pids.to_str().unwrap(),
            good = GOOD,
        );
        fs::write(&claude, script).unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            _root: dir,
            root,
            project,
            grade,
            home,
            claude,
            record,
            pids,
        }
    }

    fn env(&self) -> Vec<(OsString, OsString)> {
        [
            ("HOME", self.home.to_str().unwrap()),
            ("USER", "tester"),
            ("LOGNAME", "tester"),
            ("LANG", "en_US.UTF-8"),
            ("LC_ALL", "C"),
            ("LC_CTYPE", "UTF-8"),
            ("TMPDIR", self.root.to_str().unwrap()),
            ("PATH", "/usr/bin:relative/bin:/bin"),
            ("ANTHROPIC_API_KEY", "sk-test-not-a-key"),
            ("ANTHROPIC_BASE_URL", "http://127.0.0.1:9"),
            ("CLAUDECODE", "1"),
            ("CLAUDE_CODE_ENTRYPOINT", "cli"),
            ("CLAUDE_CODE_SSE_PORT", "1"),
            ("XTRACE_GRADE_DIR", "/somewhere/else"),
            ("DISABLE_AUTOUPDATER", "0"),
            ("SSH_AUTH_SOCK", "/tmp/agent"),
            ("FOO", "bar"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    }

    fn run_with(
        &self,
        limits: Limits,
        cancel: Option<&CancelFlag>,
    ) -> Result<xt_judge::GradeResult, GradeError> {
        let skill = bundled();
        let env = self.env();
        // The fake is named by the override, exactly as XTRACE_CLAUDE does.
        let claude = xt_probes::executable::resolve_claude(
            Some(self.claude.as_os_str()),
            runner::inherited_var(&env, "PATH"),
            Some(&self.home),
        )
        .unwrap();
        runner::run(
            &GradeRun {
                claude: &claude,
                skill: &skill,
                project: &self.project,
                grade_dir: &self.grade,
                inherited_env: &env,
                limits,
                leader_record: None,
            },
            cancel,
        )
    }

    fn run(&self) -> Result<xt_judge::GradeResult, GradeError> {
        self.run_with(quick(Duration::from_secs(60)), None)
    }

    /// A run that records its Claude Code process at `leader`.
    fn run_recording(&self, leader: &Path) -> Result<xt_judge::GradeResult, GradeError> {
        let skill = bundled();
        let env = self.env();
        let claude = xt_probes::executable::resolve_claude(
            Some(self.claude.as_os_str()),
            runner::inherited_var(&env, "PATH"),
            Some(&self.home),
        )
        .unwrap();
        runner::run(
            &GradeRun {
                claude: &claude,
                skill: &skill,
                project: &self.project,
                grade_dir: &self.grade,
                inherited_env: &env,
                limits: quick(Duration::from_secs(60)),
                leader_record: Some(leader),
            },
            None,
        )
    }

    fn record(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(&self.record).unwrap()).unwrap()
    }

    fn pids(&self) -> Vec<String> {
        fs::read_to_string(&self.pids)
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect()
    }
}

fn quick(deadline: Duration) -> Limits {
    Limits {
        deadline,
        exit_grace: Duration::from_secs(1),
        ..Limits::default()
    }
}

fn alive(pid: &str) -> bool {
    std::process::Command::new("/bin/kill")
        .args(["-0", pid])
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap()
        .success()
}

fn all_gone(pids: &[String]) -> bool {
    let until = Instant::now() + Duration::from_secs(5);
    while Instant::now() < until {
        if pids.iter().all(|pid| !alive(pid)) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

const WRITE_GOOD: &str = "write('report.html', '<p>report</p>')\nwrite('result.json', GOOD)";

#[test]
fn good_run_returns_the_validated_result() {
    let case = Case::new(WRITE_GOOD);
    let result = case.run().unwrap();
    assert_eq!(result.score, 75);
    assert_eq!(result.summary, "One dead reference.");
    assert_eq!(result.insights.len(), 2);
    assert_eq!(result.insights[0].severity, Severity::Critical);
    assert_eq!(result.insights[0].fix.as_deref(), Some("Remove it."));
    assert_eq!(result.report.as_deref(), Some("report.html"));
    assert!(result.complete);
    assert!(case.grade.join("work").is_dir());
}

/// The read-deny list: well-known secret places, each home-relative and
/// given to both the sandbox and the file tools, with nothing the bundled
/// skill reads covered.
#[test]
fn settings_deny_reading_secret_locations_and_spare_what_the_skill_reads() {
    let parsed: serde_json::Value = serde_json::from_str(&runner::settings("/p", "/g")).unwrap();
    let deny_read: Vec<&str> = parsed["sandbox"]["filesystem"]["denyRead"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry.as_str().unwrap())
        .collect();
    assert_eq!(deny_read, runner::SECRET_LOCATIONS);
    for secret in [
        "~/.ssh",
        "~/.aws",
        "~/.gnupg",
        "~/.config/gh",
        "~/Library/Keychains",
        "~/Library/Application Support/Google/Chrome",
        "~/.claude/.credentials.json",
    ] {
        assert!(deny_read.contains(&secret), "{secret}");
    }
    let rules: Vec<&str> = parsed["permissions"]["deny"]
        .as_array()
        .unwrap()
        .iter()
        .map(|rule| rule.as_str().unwrap())
        .collect();
    assert_eq!(rules.len(), 2 * runner::SECRET_LOCATIONS.len());
    for location in runner::SECRET_LOCATIONS {
        // Home-relative, one plain location each: no glob, no trailing
        // slash, no `..`, no repeat.
        assert!(location.starts_with("~/"), "{location}");
        assert!(!location.ends_with('/'), "{location}");
        assert!(!location.contains(['*', '?', '[']), "{location}");
        assert!(!location.contains(".."), "{location}");
        assert_eq!(
            deny_read.iter().filter(|entry| **entry == location).count(),
            1,
            "{location}"
        );
        assert!(rules.contains(&format!("Read({location})").as_str()));
        assert!(rules.contains(&format!("Read({location}/**)").as_str()));
    }
    // What the bundled claude-md-doctor reads stays readable: no location
    // is one of these or a folder holding one.
    for needed in [
        "~/.claude",
        "~/.claude/projects",
        "~/.claude/CLAUDE.md",
        "~/.claude/rules",
        "~/.claude/settings.json",
    ] {
        for location in runner::SECRET_LOCATIONS {
            assert!(
                !(needed == location || needed.starts_with(&format!("{location}/"))),
                "{location} covers {needed}"
            );
        }
    }
}

#[test]
fn command_line_environment_and_folder_are_exact() {
    let case = Case::new(WRITE_GOOD);
    case.run().unwrap();
    let record = case.record();
    let skill = bundled();
    let (grade, skill_dir) = (case.grade.to_str().unwrap(), skill.dir().to_str().unwrap());
    let argv: Vec<String> = record["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| arg.as_str().unwrap().to_owned())
        .collect();
    let project = case.project.to_str().unwrap();
    let prompt = skill.prompt(project, grade, &format!("{grade}/work"));
    let (deny_read, read_rules) = (
        serde_json::to_string(&runner::SECRET_LOCATIONS[..]).unwrap(),
        serde_json::to_string(&runner::secret_read_rules()).unwrap(),
    );
    let settings = format!(
        r#"{{"disableAllHooks":true,"permissions":{{"deny":{read_rules}}},"sandbox":{{"enabled":true,"failIfUnavailable":true,"allowUnsandboxedCommands":false,"filesystem":{{"denyWrite":["{project}"],"allowWrite":["{grade}"],"denyRead":{deny_read}}},"network":{{"allowedDomains":[]}}}}}}"#
    );
    let expected: Vec<String> = [
        case.claude.to_str().unwrap(),
        "-p",
        &prompt,
        "--tools",
        "Read,Write,Edit,Bash,Glob,Grep",
        "--permission-mode",
        "dontAsk",
        "--setting-sources",
        "",
        "--settings",
        &settings,
        "--strict-mcp-config",
        "--no-session-persistence",
        "--allowedTools",
        &format!("Edit(/{grade}/**)"),
        "--disallowedTools",
        &format!("Edit(/{project}/**)"),
        "--add-dir",
        grade,
        skill_dir,
    ]
    .iter()
    .map(|arg| (*arg).to_owned())
    .collect();
    assert_eq!(argv, expected);
    assert!(argv[14].starts_with("Edit(//"));
    assert!(argv[16].starts_with("Edit(//"));
    // The settings are JSON naming exactly the project and the grade folder.
    let parsed: serde_json::Value = serde_json::from_str(&argv[10]).unwrap();
    assert_eq!(
        parsed["sandbox"]["filesystem"]["denyWrite"],
        serde_json::json!([project])
    );
    assert_eq!(
        parsed["sandbox"]["filesystem"]["allowWrite"],
        serde_json::json!([grade])
    );
    assert_eq!(
        parsed["sandbox"]["network"]["allowedDomains"],
        serde_json::json!([])
    );
    assert_eq!(parsed["disableAllHooks"], true);
    // The secret locations are unreadable to commands and to the file tools.
    assert_eq!(
        parsed["sandbox"]["filesystem"]["denyRead"],
        serde_json::json!(runner::SECRET_LOCATIONS[..])
    );
    assert_eq!(
        parsed["permissions"]["deny"],
        serde_json::json!(runner::secret_read_rules())
    );
    // No command needs a rule of its own: the sandbox contains them all.
    assert!(!argv.iter().any(|arg| arg.starts_with("Bash(")));
    assert!(prompt.starts_with(&format!(
        "XTrace grading run (headless). REPO = {project}. WORK = {grade}/work. GRADE = {grade}."
    )));
    assert!(!prompt.contains("${CLAUDE_SKILL_DIR}"));
    assert!(prompt.contains(&format!("`SCRIPTS` = `{skill_dir}/scripts`")));
    assert!(prompt.contains(&format!("Read the rubric at {skill_dir}/xtrace-rubric.md.")));
    assert!(prompt.contains(&format!("Write {grade}/result.json")));

    assert_eq!(record["cwd"], case.project.to_str().unwrap());

    let mut env: Vec<(String, String)> = record["env"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(key, value)| (key.clone(), value.as_str().unwrap().to_owned()))
        // Added by the host, not by the runner: macOS's CoreFoundation adds
        // the first to every process; Apple's /usr/bin/python3 launcher adds
        // the developer-tools variables before the fake's code runs.
        .filter(|(key, _)| {
            key != "__CF_USER_TEXT_ENCODING"
                && !(python3() == Path::new("/usr/bin/python3")
                    && ["CPATH", "LIBRARY_PATH", "MANPATH", "SDKROOT"].contains(&key.as_str()))
        })
        .collect();
    env.sort();
    let bin = case.claude.parent().unwrap().to_str().unwrap();
    let mut expected_env: Vec<(String, String)> = [
        ("DISABLE_AUTOUPDATER", "1"),
        ("HOME", case.home.to_str().unwrap()),
        ("LANG", "en_US.UTF-8"),
        ("LC_ALL", "C"),
        ("LC_CTYPE", "UTF-8"),
        ("LOGNAME", "tester"),
        (
            "PATH",
            &format!("{bin}:/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin"),
        ),
        ("PYTHONDONTWRITEBYTECODE", "1"),
        ("TMPDIR", case.root.to_str().unwrap()),
        ("USER", "tester"),
        ("XTRACE_GRADE_DIR", grade),
    ]
    .iter()
    .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
    .collect();
    expected_env.sort();
    assert_eq!(env, expected_env);
}

#[test]
fn bad_results_fail_with_typed_reasons() {
    let oversized = format!(
        "write('result.json', '{{\"protocol\": 1, \"summary\": \"' + 'x' * {} + '\"}}')",
        2 * 1024 * 1024
    );
    let cases: [(&str, ResultError); 12] = [
        ("pass", ResultError::Missing),
        ("write('result.json', 'not json')", ResultError::NotJson),
        ("write('result.json', '[1, 2]')", ResultError::NotObject),
        (
            "write('result.json', GOOD.replace('75', '101'))",
            ResultError::Score,
        ),
        (
            "write('result.json', GOOD.replace('75', '-5'))",
            ResultError::Score,
        ),
        (&oversized, ResultError::TooLarge),
        (
            "open(os.path.join(os.path.dirname(G), 'escape.html'), 'w').write('x')\n\
             write('result.json', GOOD.replace('report.html', '../escape.html'))",
            ResultError::ReportPath,
        ),
        (
            "os.symlink(os.path.join(os.path.dirname(G), 'record.json'), os.path.join(G, 'report.html'))\n\
             write('result.json', GOOD)",
            ResultError::ReportOutside,
        ),
        (
            "r = json.loads(GOOD)\ndel r['criteria']\nwrite('report.html', 'x')\nwrite('result.json', json.dumps(r))",
            ResultError::CriteriaRubric,
        ),
        (
            "write('report.html', 'x')\nwrite('result.json', GOOD.replace('Health of the memory files', 'Health'))",
            ResultError::CriteriaRubric,
        ),
        (
            "write('report.html', 'x')\nwrite('result.json', GOOD.replace('\"awarded\": 75', '\"awarded\": 70'))",
            ResultError::CriteriaTotal,
        ),
        (
            "write('report.html', 'x')\nwrite('result.json', GOOD.replace('\"criteria\": [', '\"criteria\": [1, '))",
            ResultError::Criteria,
        ),
    ];
    for (body, expected) in cases {
        let case = Case::new(body);
        assert_eq!(case.run(), Err(GradeError::Result(expected)), "{body}");
    }
}

#[test]
fn a_failed_exit_fails_even_with_a_result() {
    let case = Case::new(&format!("{WRITE_GOOD}\nsys.exit(3)"));
    assert_eq!(case.run(), Err(GradeError::Exited { code: Some(3) }));
}

#[test]
fn claude_code_at_or_above_the_probed_version_runs() {
    for version in [
        "2.1.296 (Claude Code)",
        "2.1.300 (Claude Code)",
        "2.2.0",
        "10.0.0 (Claude Code)",
    ] {
        let case = Case::with_version(&format!("    print({version:?})"), WRITE_GOOD);
        assert_eq!(case.run().unwrap().score, 75, "{version}");
    }
}

#[test]
fn an_older_or_unreadable_claude_code_is_refused_before_the_run() {
    for answer in [
        "    print('2.1.295 (Claude Code)')",
        "    print('1.9.999')",
        "    print('Claude Code')",
        "    print('2.1')",
        "    print('2.1.296.1')",
        "    print('v2.1.296')",
        "    pass",
        "    print('2.1.296 (Claude Code)')\n    sys.exit(1)",
    ] {
        let case = Case::with_version(answer, WRITE_GOOD);
        assert_eq!(case.run(), Err(GradeError::ClaudeTooOld), "{answer}");
        assert!(!case.record.exists(), "the run never started: {answer}");
        assert_eq!(
            fs::read_dir(&case.grade).unwrap().count(),
            0,
            "nothing was prepared: {answer}"
        );
    }
    assert!(!GradeError::ClaudeTooOld.to_string().contains('/'));
}

#[test]
fn version_output_is_read_strictly() {
    use xt_judge::runner::{MIN_CLAUDE_VERSION, parse_claude_version};
    assert_eq!(MIN_CLAUDE_VERSION, (2, 1, 296));
    assert_eq!(
        parse_claude_version(b"2.1.296 (Claude Code)\n"),
        Some((2, 1, 296))
    );
    assert_eq!(parse_claude_version(b"  3.0.1\n"), Some((3, 0, 1)));
    for bad in [
        &b""[..],
        b"2.1",
        b"2.1.x",
        b"2..1",
        b"2.1.296-beta",
        b"\xff",
        b"(Claude Code)",
    ] {
        assert_eq!(parse_claude_version(bad), None, "{bad:?}");
    }
}

#[test]
fn a_cancel_during_the_version_check_ends_it() {
    let case = Case::with_version("    time.sleep(600)", WRITE_GOOD);
    let cancel = CancelFlag::new();
    let trigger = cancel.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        trigger.cancel();
    });
    let started = Instant::now();
    assert_eq!(
        case.run_with(quick(Duration::from_secs(120)), Some(&cancel)),
        Err(GradeError::Cancelled)
    );
    canceller.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(20));
    assert!(!case.record.exists());
}

/// What Claude Code 2.1.296 writes when `failIfUnavailable` stops it.
const SANDBOX_REFUSAL: &str = "sys.stderr.write('\\nError: sandbox required but unavailable: \
     sandbox-exec not found\\n  sandbox.failIfUnavailable is set \\u2014 refusing to start \
     without a working sandbox.\\n')\nsys.exit(1)";

#[test]
fn a_missing_sandbox_fails_with_its_own_reason() {
    let case = Case::new(SANDBOX_REFUSAL);
    assert_eq!(case.run(), Err(GradeError::SandboxUnavailable));
    // The fake read the settings that require the sandbox.
    let argv = case.record()["argv"].clone();
    let settings: serde_json::Value = serde_json::from_str(argv[10].as_str().unwrap()).unwrap();
    assert_eq!(settings["sandbox"]["failIfUnavailable"], true);
    assert_eq!(settings["sandbox"]["allowUnsandboxedCommands"], false);
    // Another failure that mentions a sandbox is still a failed exit.
    let other = Case::new("sys.stderr.write('the sandbox is fine; API error')\nsys.exit(1)");
    assert_eq!(other.run(), Err(GradeError::Exited { code: Some(1) }));
    // A refusal text with a successful exit is no refusal.
    let fine = Case::new(&format!(
        "sys.stderr.write('sandbox required but unavailable')\n{WRITE_GOOD}"
    ));
    assert_eq!(fine.run().unwrap().score, 75);
    assert!(!GradeError::SandboxUnavailable.to_string().contains('/'));
}

#[test]
fn writing_into_the_project_fails_the_run_and_deletes_nothing() {
    let case = Case::new(&format!(
        "os.makedirs(os.path.join(os.getcwd(), '.claude-md-doctor', 'work'))\n{WRITE_GOOD}"
    ));
    assert_eq!(case.run(), Err(GradeError::ProjectWritten));
    assert!(case.project.join(".claude-md-doctor/work").is_dir());
}

/// A `.claude-md-doctor` folder the user already had, with one file in it.
fn existing_work_folder(case: &Case) -> PathBuf {
    let folder = case.project.join(".claude-md-doctor");
    fs::create_dir_all(folder.join("notes")).unwrap();
    fs::write(folder.join("notes/mine.txt"), "the user's own").unwrap();
    folder
}

#[test]
fn an_existing_work_folder_left_unchanged_passes() {
    let case = Case::new(WRITE_GOOD);
    let folder = existing_work_folder(&case);
    assert_eq!(case.run().unwrap().score, 75);
    assert_eq!(
        fs::read_to_string(folder.join("notes/mine.txt")).unwrap(),
        "the user's own"
    );
}

#[test]
fn any_change_to_an_existing_work_folder_fails_the_run_and_deletes_nothing() {
    for change in [
        "open(os.path.join(os.getcwd(), '.claude-md-doctor', 'new.txt'), 'w').close()",
        "open(os.path.join(os.getcwd(), '.claude-md-doctor', 'notes', 'mine.txt'), 'a').write('!')",
        "os.makedirs(os.path.join(os.getcwd(), '.claude-md-doctor', 'work'))",
    ] {
        let case = Case::new(&format!("{change}\n{WRITE_GOOD}"));
        let folder = existing_work_folder(&case);
        assert_eq!(case.run(), Err(GradeError::ProjectWritten), "{change}");
        assert!(folder.join("notes/mine.txt").is_file(), "{change}");
    }
}

#[test]
fn an_existing_work_folder_too_large_to_check_refuses_the_run_before_it_starts() {
    let case = Case::new(WRITE_GOOD);
    let folder = existing_work_folder(&case);
    for index in 0..runner::MAX_SNAPSHOT_ENTRIES {
        fs::write(folder.join(format!("f{index}")), "").unwrap();
    }
    assert_eq!(case.run(), Err(GradeError::ProjectWorkFolderUncheckable));
    assert!(!case.record.exists(), "the fake never ran");
    assert!(folder.join("notes/mine.txt").is_file());
}

#[test]
fn a_work_folder_that_is_a_link_refuses_the_run_before_it_starts() {
    for target_is_folder in [true, false] {
        let case = Case::new(WRITE_GOOD);
        let target = case.root.join("elsewhere");
        if target_is_folder {
            fs::create_dir(&target).unwrap();
            fs::write(target.join("theirs.txt"), "kept").unwrap();
        } else {
            fs::write(&target, "kept").unwrap();
        }
        let link = case.project.join(".claude-md-doctor");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert_eq!(case.run(), Err(GradeError::ProjectWorkFolderUncheckable));
        assert!(!case.record.exists(), "the fake never ran");
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(target.exists(), "nothing is deleted");
    }
}

const HANG: &str = "child = subprocess.Popen(['/bin/sleep', '600'])\n\
     with open(PIDS, 'w') as f:\n    f.write('%d %d' % (os.getpid(), child.pid))\n\
     time.sleep(600)";

#[test]
fn deadline_kills_the_group_and_reaps_it() {
    let case = Case::new(HANG);
    let started = Instant::now();
    assert_eq!(
        case.run_with(quick(Duration::from_secs(2)), None),
        Err(GradeError::Timeout)
    );
    assert!(started.elapsed() < Duration::from_secs(20));
    let pids = case.pids();
    assert_eq!(pids.len(), 2, "the fake recorded its pids");
    assert!(all_gone(&pids), "every process of the run is gone");
}

#[test]
fn cancel_kills_the_group_and_reaps_it() {
    let case = Case::new(HANG);
    let cancel = CancelFlag::new();
    let trigger = cancel.clone();
    let pids_file = case.pids.clone();
    let canceller = std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(20);
        while !pids_file.exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        trigger.cancel();
    });
    let started = Instant::now();
    assert_eq!(
        case.run_with(quick(Duration::from_secs(120)), Some(&cancel)),
        Err(GradeError::Cancelled)
    );
    canceller.join().unwrap();
    assert!(started.elapsed() < Duration::from_secs(30));
    assert!(all_gone(&case.pids()));
}

#[test]
fn a_cancel_before_the_start_runs_nothing() {
    let case = Case::new(WRITE_GOOD);
    let cancel = CancelFlag::new();
    cancel.cancel();
    assert_eq!(
        case.run_with(quick(Duration::from_secs(60)), Some(&cancel)),
        Err(GradeError::Cancelled)
    );
    assert!(!case.record.exists());
}

#[test]
fn a_leftover_descendant_holding_the_pipes_is_ended_after_exit() {
    let case = Case::new(&format!(
        "child = subprocess.Popen(['/bin/sleep', '600'])\n\
         with open(PIDS, 'w') as f:\n    f.write('%d' % child.pid)\n{WRITE_GOOD}"
    ));
    let started = Instant::now();
    assert_eq!(case.run().unwrap().score, 75);
    assert!(started.elapsed() < Duration::from_secs(20));
    assert!(all_gone(&case.pids()));
}

#[test]
fn a_left_behind_member_cannot_write_after_a_normal_finish() {
    // The member redirects its output, so the pipes close when the fake
    // exits and the run finishes normally while the member still sleeps.
    let case = Case::new(&format!(
        "subprocess.Popen(['/bin/sh', '-c', 'sleep 4; echo late > \"$G/late.txt\"'],\n\
         \x20   env=dict(os.environ, G=G), stdin=subprocess.DEVNULL,\n\
         \x20   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n\
         with open(PIDS, 'w') as f:\n    f.write('%d' % os.getpid())\n{WRITE_GOOD}"
    ));
    let started = Instant::now();
    assert_eq!(case.run().unwrap().score, 75);
    assert!(started.elapsed() < Duration::from_secs(4), "not held open");
    // The fake led its own process group; its pid is the group's ID.
    let pids = case.pids();
    assert_eq!(pids.len(), 1);
    let group_alive = || {
        std::process::Command::new("/usr/bin/pgrep")
            .args(["-g", &pids[0]])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };
    let until = Instant::now() + Duration::from_secs(5);
    while group_alive() && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(!group_alive(), "no member of the run is alive");
    std::thread::sleep(Duration::from_secs(5).saturating_sub(started.elapsed()));
    std::thread::sleep(Duration::from_secs(1));
    assert!(
        !case.grade.join("late.txt").exists(),
        "the member never wrote"
    );
}

#[test]
fn folder_paths_with_whitespace_are_refused_before_anything_runs() {
    let case = Case::new(WRITE_GOOD);
    let env = case.env();
    let spaced_grade = case.root.join("grade two");
    fs::create_dir(&spaced_grade).unwrap();
    let skill = bundled();
    let attempt = |skill: &GradingSkill, grade: &Path| {
        runner::run(
            &GradeRun {
                claude: &case.claude,
                skill,
                project: &case.project,
                grade_dir: grade,
                inherited_env: &env,
                limits: Limits::default(),
                leader_record: None,
            },
            None,
        )
    };
    assert_eq!(
        attempt(&skill, &spaced_grade),
        Err(GradeError::GradeDirPathHasWhitespace)
    );
    assert_eq!(fs::read_dir(&spaced_grade).unwrap().count(), 0);

    // A copy of the bundled skill in a folder whose name has a space.
    let copy = case.root.join("skill copy");
    let source = skill.dir();
    fs::create_dir_all(copy.join("scripts")).unwrap();
    for name in ["SKILL.md", "xtrace-rubric.md"] {
        fs::copy(source.join(name), copy.join(name)).unwrap();
    }
    let spaced_skill = GradingSkill::open(&copy).unwrap();
    assert_eq!(
        attempt(&spaced_skill, &case.grade),
        Err(GradeError::SkillPathHasWhitespace)
    );
    for error in [
        GradeError::SkillPathHasWhitespace,
        GradeError::GradeDirPathHasWhitespace,
    ] {
        assert!(!error.to_string().contains('/'));
    }
    assert!(!case.record.exists());
    assert_eq!(fs::read_dir(&case.grade).unwrap().count(), 0);
}

#[test]
fn folders_are_checked_before_anything_runs() {
    let case = Case::new(WRITE_GOOD);
    fs::write(case.grade.join("stale.json"), "{}").unwrap();
    assert_eq!(case.run(), Err(GradeError::GradeDirNotEmpty));
    assert!(!case.record.exists());
    fs::remove_file(case.grade.join("stale.json")).unwrap();

    let skill = bundled();
    let env = case.env();
    let inside = case.project.join("grades");
    fs::create_dir(&inside).unwrap();
    let attempt = |claude: &Path, project: &Path, grade: &Path| {
        runner::run(
            &GradeRun {
                claude,
                skill: &skill,
                project,
                grade_dir: grade,
                inherited_env: &env,
                limits: Limits::default(),
                leader_record: None,
            },
            None,
        )
    };
    assert_eq!(
        attempt(&case.claude, &case.project, &inside),
        Err(GradeError::GradeDirOverlapsProject)
    );
    assert_eq!(
        attempt(&case.claude, &case.root, &case.grade),
        Err(GradeError::GradeDirOverlapsProject)
    );
    assert_eq!(
        attempt(&case.claude, &case.root.join("missing"), &case.grade),
        Err(GradeError::ProjectInvalid)
    );
    assert_eq!(
        attempt(Path::new("claude"), &case.project, &case.grade),
        Err(GradeError::ClaudeNotAbsolute)
    );
    assert!(!case.record.exists());
}

#[test]
fn the_override_must_name_an_executable() {
    use xt_probes::executable::{Unavailable, resolve_claude};
    let case = Case::new(WRITE_GOOD);
    assert_eq!(
        resolve_claude(Some(OsStr::new("claude")), None, None),
        Err(Unavailable::OverrideNotAbsolute)
    );
    assert_eq!(
        resolve_claude(Some(case.record.as_os_str()), None, None),
        Err(Unavailable::OverrideNotExecutable)
    );
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn the_launched_claude_is_recorded_as_its_group_leader() {
    let case = Case::new(&format!(
        "with open(PIDS, 'a') as f:\n    f.write(str(os.getpid()) + ' ')\n{WRITE_GOOD}"
    ));
    let leader = case.root.join("leader");
    case.run_recording(&leader).unwrap();
    let recorded =
        xt_probes::leader::GroupLeader::parse(&fs::read(&leader).unwrap()).expect("a record");
    assert_eq!(case.pids(), [recorded.pid.to_string()]);
    assert_eq!(
        fs::metadata(&leader).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // The run is over and its group gone: the record names no live process.
    assert!(!recorded.is_running());
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_record_that_cannot_be_written_ends_the_run_at_once() {
    let case = Case::new(
        "with open(PIDS, 'a') as f:\n    f.write(str(os.getpid()) + ' ')\ntime.sleep(30)\nwrite('result.json', GOOD)",
    );
    let leader = case.root.join("leader");
    // Already there: a record is never written over another.
    fs::write(&leader, "taken").unwrap();
    let started = Instant::now();
    assert_eq!(case.run_recording(&leader), Err(GradeError::Prepare));
    assert!(started.elapsed() < Duration::from_secs(15));
    assert_eq!(fs::read_to_string(&leader).unwrap(), "taken");
    assert!(all_gone(&case.pids()));
}
