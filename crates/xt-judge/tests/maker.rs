//! Rubric-making runs against a fake `claude`.
#![cfg(unix)]
//!
//! Every case writes a synthetic `claude` (a small Python program) and a
//! synthetic installed skill into a fresh folder under Cargo's target temp
//! folder. The real Claude Code CLI is never run; the real home folder and
//! XTrace's data folders are never read.

use std::{
    ffi::OsString,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use xt_judge::{
    CancelFlag, GradeError, Limits, RubricError,
    maker::{self, Existing, Helper, MakeError, MakeRun, MakerStaging, TryProblem, Written},
    rubric::{self, RUBRIC_FILE},
    runner,
};

fn helper_dir() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/rubric-maker"))
}

fn python3() -> PathBuf {
    xt_probes::executable::resolve_within(
        "python3",
        None,
        std::env::var_os("PATH").as_deref(),
        &[Path::new("/usr/bin"), Path::new("/opt/homebrew/bin")],
    )
    .expect("python3 is required for these tests")
}

const RUN_ID: &str = "0f8fad5b-d9cb-469f-a165-70867728950e";
const KEY: &str = "0123456789abcdef";

/// A valid rubric the fake can write.
const GOOD: &str = "---\nprotocol: 1\nid: readme-check\nname: README Check\nversion: 1\n\
     description: How well the README explains the project.\ntarget: project\n---\n\
     ## Install steps (60 points)\nAward 60 when ok.\n\n## Usage (40 points)\nAward 40 when ok.\n";
/// A rubric whose points add up to 90.
const SHORT: &str = "---\nprotocol: 1\nid: readme-check\nname: README Check\nversion: 1\n\
     description: d\ntarget: project\n---\n## All (90 points)\nAll.\n";

struct Case {
    _root: tempfile::TempDir,
    root: PathBuf,
    temp: PathBuf,
    target: PathBuf,
    home: PathBuf,
    claude: PathBuf,
    record: PathBuf,
    pids: PathBuf,
    ready: PathBuf,
}

impl Case {
    /// A fake whose behaviour is `body`, Python run after it has appended
    /// its argv, environment and working folder to the record. `OUT` is the
    /// output folder from its environment and `TRY` its name (`out`, then
    /// `out-2`).
    fn new(body: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("maker-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let root = fs::canonicalize(dir.path()).unwrap();
        let (temp, target, home, bin) = (
            root.join("tmp"),
            root.join("home/.claude/skills/readme"),
            root.join("home"),
            root.join("bin"),
        );
        for folder in [&temp, &target, &home, &bin] {
            fs::create_dir_all(folder).unwrap();
        }
        fs::write(
            target.join("SKILL.md"),
            "---\nname: readme\ndescription: Checks READMEs.\n---\nRead the README.\n",
        )
        .unwrap();
        fs::create_dir(target.join("scripts")).unwrap();
        fs::write(target.join("scripts/check.py"), "print('ok')\n").unwrap();
        let (record, pids, ready) = (
            root.join("record.jsonl"),
            root.join("pids"),
            root.join("ready"),
        );
        let claude = bin.join("claude");
        let script = format!(
            "#!{python} -I\nimport json, os, subprocess, sys, time\n\
             RECORD = {record:?}\nPIDS = {pids:?}\nREADY = {ready:?}\n\
             if sys.argv[1:] == ['--version']:\n    print('2.1.296 (Claude Code)')\n    sys.exit(0)\n\
             with open(RECORD, 'a') as f:\n    f.write(json.dumps({{'argv': sys.argv, 'env': dict(os.environ), 'cwd': os.getcwd()}}) + '\\n')\n\
             OUT = os.environ['XTRACE_GRADE_DIR']\nTRY = os.path.basename(OUT)\n\
             def write(text, name='xtrace-rubric.md'):\n    with open(os.path.join(OUT, name), 'w') as f:\n        f.write(text)\n\
             GOOD = {good:?}\nSHORT = {short:?}\n{body}\n",
            python = python3().display(),
            record = record.to_str().unwrap(),
            pids = pids.to_str().unwrap(),
            ready = ready.to_str().unwrap(),
            good = GOOD,
            short = SHORT,
        );
        fs::write(&claude, script).unwrap();
        fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            _root: dir,
            root,
            temp,
            target,
            home,
            claude,
            record,
            pids,
            ready,
        }
    }

    fn env(&self) -> Vec<(OsString, OsString)> {
        [
            ("HOME", self.home.to_str().unwrap()),
            ("USER", "tester"),
            ("LANG", "en_US.UTF-8"),
            ("TMPDIR", self.temp.to_str().unwrap()),
            ("PATH", "/usr/bin:/bin"),
            ("ANTHROPIC_API_KEY", "sk-test-not-a-key"),
            ("CLAUDECODE", "1"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect()
    }

    fn staging(&self) -> MakerStaging {
        MakerStaging::create(&self.temp, KEY, RUN_ID, &self.target, &helper_dir(), None).unwrap()
    }

    fn run_in(
        &self,
        staging: &MakerStaging,
        existing: Option<RubricError>,
        limits: Limits,
        cancel: Option<&CancelFlag>,
    ) -> Result<Written, MakeError> {
        maker::run(
            &MakeRun {
                claude: &self.claude,
                staging,
                inherited_env: &self.env(),
                limits,
                existing,
            },
            cancel,
        )
    }

    fn run(&self) -> (MakerStaging, Result<Written, MakeError>) {
        let staging = self.staging();
        let made = self.run_in(&staging, None, quick(Duration::from_secs(60)), None);
        (staging, made)
    }

    fn records(&self) -> Vec<serde_json::Value> {
        fs::read_to_string(&self.record)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn argv(record: &serde_json::Value) -> Vec<String> {
        record["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg.as_str().unwrap().to_owned())
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

#[test]
fn the_bundled_helper_opens_and_its_worked_example_is_a_valid_rubric() {
    let helper = Helper::open(&helper_dir()).unwrap();
    let text = helper.instructions();
    for rule in [
        "OUT/xtrace-rubric.md",
        "protocol: 1",
        "target: project",
        "add up to exactly 100",
        "## Findings",
        "Do not run the skill",
        "OUT/xtrace-refusal.md",
        "## How XTrace uses your rubric",
        "Never invent a new",
        "300 characters",
        "Usually that is 0.",
    ] {
        assert!(text.contains(rule), "{rule}");
    }
    let example = text
        .split_once("```markdown\n")
        .and_then(|(_, rest)| rest.split_once("\n```"))
        .map(|(example, _)| format!("{example}\n"))
        .unwrap();
    let rubric = rubric::parse(example.as_bytes()).unwrap();
    assert_eq!(rubric.id, "readme-check");
    assert_eq!(
        rubric
            .criteria
            .iter()
            .map(|criterion| criterion.points)
            .sum::<u8>(),
        100
    );
    assert!(rubric.findings.is_some());
}

#[test]
fn a_good_first_try_is_read_back_with_its_exact_bytes() {
    let case = Case::new("write(GOOD)");
    let (staging, made) = case.run();
    let made = made.unwrap().made().unwrap();
    assert_eq!(made.tries, 1);
    assert_eq!(made.bytes, GOOD.as_bytes());
    assert_eq!(made.rubric.id, "readme-check");
    assert_eq!(case.records().len(), 1);
    // The skill itself was not touched; only the staged copy was used.
    assert!(!case.target.join(RUBRIC_FILE).exists());
    assert!(!staging.leader_record().exists(), "the record is cleared");
}

/// The exact command line: the grading run's, with the staged copy of the
/// skill as the read-only working folder, the output folder the only
/// writable one and the staged helper as the skill folder.
#[test]
fn the_command_line_environment_and_folder_are_the_grading_runs() {
    let case = Case::new("write(GOOD)");
    let (staging, made) = case.run();
    made.unwrap();
    let record = &case.records()[0];
    let argv = Case::argv(record);
    if std::env::var_os("XTRACE_PRINT_MAKER_ARGV").is_some() {
        println!("{}", serde_json::to_string_pretty(&record["argv"]).unwrap());
    }
    let target = fs::canonicalize(staging.target_dir()).unwrap();
    let out = fs::canonicalize(staging.out_dir(0)).unwrap();
    let helper = fs::canonicalize(staging.helper_dir()).unwrap();
    let (target, out, helper) = (
        target.to_str().unwrap(),
        out.to_str().unwrap(),
        helper.to_str().unwrap(),
    );
    let prompt = maker::prompt(
        &Helper::open(Path::new(helper)).unwrap(),
        target,
        out,
        None,
        None,
    );
    let (deny_read, read_rules) = (
        serde_json::to_string(&runner::SECRET_LOCATIONS[..]).unwrap(),
        serde_json::to_string(&runner::secret_read_rules()).unwrap(),
    );
    let settings = format!(
        r#"{{"disableAllHooks":true,"permissions":{{"deny":{read_rules}}},"sandbox":{{"enabled":true,"failIfUnavailable":true,"allowUnsandboxedCommands":false,"filesystem":{{"denyWrite":["{target}"],"allowWrite":["{out}"],"denyRead":{deny_read}}},"network":{{"allowedDomains":[]}}}}}}"#
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
        &format!("Edit(/{out}/**)"),
        "--disallowedTools",
        &format!("Edit(/{target}/**)"),
        "--add-dir",
        out,
        helper,
    ]
    .iter()
    .map(|arg| (*arg).to_owned())
    .collect();
    assert_eq!(argv, expected);
    // Exactly the grading run's command line over these folders.
    assert_eq!(
        argv.iter().map(OsString::from).collect::<Vec<_>>(),
        runner::command_line(&case.claude, prompt.clone(), target, out, helper)
    );
    assert_eq!(record["cwd"], target);
    assert_eq!(record["env"]["XTRACE_GRADE_DIR"], out);
    for dropped in ["ANTHROPIC_API_KEY", "CLAUDECODE"] {
        assert!(record["env"].get(dropped).is_none(), "{dropped}");
    }
    for path in [target, out, helper] {
        assert!(!path.contains(char::is_whitespace));
        assert!(path.starts_with(staging.root().to_str().unwrap()));
    }
    assert!(prompt.starts_with(&format!(
        "XTrace rubric-making run (headless). TARGET = {target}. OUT = {out}.\n\
         TARGET is a copy of the skill to write a rubric for, and it is read-only. Write exactly \
         one file: {out}/xtrace-rubric.md, or {out}/xtrace-refusal.md when the helper \
         instructions say to refuse. Write nothing else anywhere."
    )));
    assert!(prompt.contains("===== Helper instructions ====="));
    assert!(prompt.contains("# XTrace rubric maker"));
    // The staged copy holds the skill's files, the helper its own.
    assert!(Path::new(target).join("scripts/check.py").is_file());
    assert!(Path::new(helper).join("SKILL.md").is_file());
}

#[test]
fn an_invalid_first_try_is_retried_once_with_the_readers_reason() {
    let case = Case::new("write(SHORT if TRY == 'out' else GOOD)");
    let (_staging, made) = case.run();
    let made = made.unwrap().made().unwrap();
    assert_eq!(made.tries, 2);
    assert_eq!(made.bytes, GOOD.as_bytes());
    let records = case.records();
    assert_eq!(records.len(), 2);
    let first = &Case::argv(&records[0])[2];
    let second = &Case::argv(&records[1])[2];
    assert!(!first.contains("previous try"));
    assert!(second.contains(&format!(
        "A previous try of this run wrote a rubric XTrace refused: {}.",
        RubricError::Total
    )));
    assert!(second.contains("the rubric's points do not add up to 100"));
    // A fresh, empty output folder for the retry.
    assert!(
        records[1]["env"]["XTRACE_GRADE_DIR"]
            .as_str()
            .unwrap()
            .ends_with("/out-2")
    );
}

#[test]
fn a_second_bad_try_fails_with_the_second_reason() {
    let case = Case::new("write(SHORT if TRY == 'out' else 'no header')");
    let (_, made) = case.run();
    assert_eq!(made, Err(MakeError::Rubric(RubricError::NoHeader)));
    assert_eq!(case.records().len(), 2);

    let case = Case::new("pass");
    let (_, made) = case.run();
    assert_eq!(made, Err(MakeError::Rubric(RubricError::Missing)));
    let records = case.records();
    assert!(Case::argv(&records[1])[2].contains("A previous try of this run wrote neither"));
}

#[test]
fn a_rubric_being_made_again_names_why_the_current_one_is_refused() {
    let case = Case::new("write(GOOD)");
    let staging = case.staging();
    case.run_in(
        &staging,
        Some(RubricError::DuplicateCriterion),
        quick(Duration::from_secs(60)),
        None,
    )
    .unwrap();
    let prompt = &Case::argv(&case.records()[0])[2];
    assert!(prompt.contains(&format!(
        "The skill's current xtrace-rubric.md is refused by XTrace: {}. It is not in TARGET.",
        RubricError::DuplicateCriterion
    )));
    let helper = Helper::open(&helper_dir()).unwrap();
    let existing = |error| Existing {
        error,
        in_target: true,
    };
    assert!(
        !maker::prompt(
            &helper,
            "/t",
            "/o",
            Some(existing(RubricError::Missing)),
            None
        )
        .contains("is refused by XTrace")
    );
    assert!(
        maker::prompt(
            &helper,
            "/t",
            "/o",
            Some(existing(RubricError::Total)),
            None
        )
        .contains(
            "The skill's current xtrace-rubric.md (a copy is in TARGET) is refused by XTrace"
        )
    );
}

/// The retry of a rubric being made again says both why the old rubric was
/// refused and what was wrong with the first try.
#[test]
fn a_retry_of_a_remake_keeps_both_reasons() {
    let case = Case::new("write(SHORT if TRY == 'out' else GOOD)");
    let staging = case.staging();
    case.run_in(
        &staging,
        Some(RubricError::NoHeader),
        quick(Duration::from_secs(60)),
        None,
    )
    .unwrap();
    let records = case.records();
    assert_eq!(records.len(), 2);
    let retry = &Case::argv(&records[1])[2];
    assert!(retry.contains(&format!(
        "The skill's current xtrace-rubric.md is refused by XTrace: {}.",
        RubricError::NoHeader
    )));
    assert!(retry.contains(&format!(
        "A previous try of this run wrote a rubric XTrace refused: {}.",
        RubricError::Total
    )));
}

#[test]
fn a_refusal_ends_the_run_with_its_reason_and_is_not_retried() {
    let case = Case::new(
        "write('  This skill publishes\\nreleases;\\tit does not review a project.\\n', 'xtrace-refusal.md')",
    );
    let (_, written) = case.run();
    assert_eq!(
        written,
        Ok(Written::Refused(maker::Refusal {
            reason: "This skill publishes releases; it does not review a project.".into(),
            tries: 1,
        }))
    );
    assert_eq!(case.records().len(), 1);

    // A long reason is cut at the limit.
    let case = Case::new("write('word ' * 200, 'xtrace-refusal.md')");
    let (_, written) = case.run();
    let Ok(Written::Refused(refusal)) = written else {
        panic!("not refused: {written:?}");
    };
    assert_eq!(refusal.reason.chars().count(), maker::MAX_REFUSAL_CHARS);
    assert!(refusal.reason.ends_with("word…"));
}

#[test]
fn both_files_or_an_unreadable_refusal_is_retried_and_reported() {
    // Both files: retried with that reason; both again fails.
    let case = Case::new("write(GOOD)\nwrite('No.', 'xtrace-refusal.md')");
    let (_, written) = case.run();
    assert_eq!(written, Err(MakeError::Both));
    let records = case.records();
    assert_eq!(records.len(), 2);
    assert!(
        Case::argv(&records[1])[2].contains(
            "A previous try of this run wrote both xtrace-rubric.md and xtrace-refusal.md."
        )
    );
    // Both, then a good rubric: made.
    let case = Case::new("write(GOOD)\nif TRY == 'out':\n    write('No.', 'xtrace-refusal.md')");
    let (_, written) = case.run();
    assert_eq!(written.unwrap().made().unwrap().tries, 2);
    // A refusal that is blank, not text, or a link is not read; the retry
    // says so, and the last one's problem is the run's.
    for body in [
        "write('   \\n', 'xtrace-refusal.md')",
        "write('bad \\x00 byte', 'xtrace-refusal.md')",
        "os.symlink('/etc/hosts', os.path.join(OUT, 'xtrace-refusal.md'))",
    ] {
        let case = Case::new(body);
        let (_, written) = case.run();
        assert_eq!(written, Err(MakeError::Refusal), "{body}");
        let records = case.records();
        assert_eq!(records.len(), 2);
        assert!(Case::argv(&records[1])[2].contains(
            "A previous try of this run wrote a xtrace-refusal.md XTrace could not read"
        ));
    }
    assert_eq!(
        MakeError::from(TryProblem::Rubric(RubricError::Total)),
        MakeError::Rubric(RubricError::Total)
    );
}

#[test]
fn a_failed_run_is_not_retried_and_keeps_its_own_reason() {
    let case = Case::new("write(GOOD)\nsys.exit(3)");
    let (_, made) = case.run();
    assert_eq!(
        made,
        Err(MakeError::Run(GradeError::Exited { code: Some(3) }))
    );
    assert_eq!(case.records().len(), 1);

    let case = Case::new(
        "sys.stderr.write('Error: sandbox required but unavailable: no\\n')\nsys.exit(1)",
    );
    let (_, made) = case.run();
    assert_eq!(made, Err(MakeError::Run(GradeError::SandboxUnavailable)));
}

#[test]
fn an_old_claude_code_is_refused_before_anything_runs() {
    let case = Case::new("write(GOOD)");
    let script = fs::read_to_string(&case.claude)
        .unwrap()
        .replace("2.1.296 (Claude Code)", "2.1.295 (Claude Code)");
    fs::write(&case.claude, script).unwrap();
    let (_, made) = case.run();
    assert_eq!(made, Err(MakeError::Run(GradeError::ClaudeTooOld)));
    assert!(case.records().is_empty());
}

#[test]
fn cancel_and_the_deadline_end_the_whole_group() {
    let hang = "child = subprocess.Popen(['/bin/sleep', '600'])\n\
                with open(PIDS, 'w') as f:\n    f.write('%d %d' % (os.getpid(), child.pid))\n\
                open(READY, 'w').close()\ntime.sleep(600)";
    let case = Case::new(hang);
    let staging = case.staging();
    let cancel = CancelFlag::new();
    let flag = cancel.clone();
    let ready = case.ready.clone();
    let waiter = std::thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(30);
        while !ready.exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
        flag.cancel();
    });
    let made = case.run_in(
        &staging,
        None,
        quick(Duration::from_secs(60)),
        Some(&cancel),
    );
    waiter.join().unwrap();
    assert_eq!(made, Err(MakeError::Run(GradeError::Cancelled)));
    let pids: Vec<String> = fs::read_to_string(&case.pids)
        .unwrap()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    assert!(pids.iter().all(|pid| !alive(pid)), "{pids:?}");
    assert_eq!(case.records().len(), 1, "a cancelled try is not retried");

    let case = Case::new(hang);
    let staging = case.staging();
    let made = case.run_in(&staging, None, quick(Duration::from_millis(1500)), None);
    assert_eq!(made, Err(MakeError::Run(GradeError::Timeout)));
}

#[test]
fn staging_copies_the_skill_within_its_limits_and_is_removed() {
    let case = Case::new("write(GOOD)");
    let staging = case.staging();
    let root = staging.root().to_path_buf();
    assert_eq!(
        root,
        fs::canonicalize(maker::maker_parent(&case.temp, KEY))
            .unwrap()
            .join(RUN_ID)
    );
    assert!(root.join("target/SKILL.md").is_file());
    assert!(root.join("helper/SKILL.md").is_file());
    assert!(staging.remove());
    assert!(!root.exists());

    // A link out of the skill's folder refuses it.
    std::os::unix::fs::symlink(case.root.join("record.jsonl"), case.target.join("leak")).unwrap();
    fs::write(case.root.join("record.jsonl"), "").unwrap();
    assert_eq!(
        MakerStaging::create(&case.temp, KEY, RUN_ID, &case.target, &helper_dir(), None)
            .map(|_| ()),
        Err(GradeError::SkillLinkRefused)
    );
    assert!(!root.exists(), "a refused copy leaves nothing");
    // Not a run ID or not a key: nothing is made.
    for (key, id) in [("../x", RUN_ID), (KEY, "../escape")] {
        assert_eq!(
            MakerStaging::create(&case.temp, key, id, &case.target, &helper_dir(), None)
                .map(|_| ()),
            Err(GradeError::StagingFailed)
        );
    }
}

#[test]
fn leftovers_of_this_app_are_stopped_and_removed_after_a_crash() {
    let case = Case::new("");
    let staging = case.staging();
    let root = staging.root().to_path_buf();
    // A crash leaves the folder and a running group behind.
    std::mem::forget(staging);
    let mut sleeper = std::process::Command::new("/bin/sleep");
    sleeper.arg("600").stdin(std::process::Stdio::null());
    let mut child = xt_probes::process::spawn_grouped(&mut sleeper).unwrap();
    let leader = xt_probes::leader::GroupLeader::observe(child.id()).unwrap();
    xt_judge::staging::write_private(&root.join("leader"), leader.record().as_bytes()).unwrap();
    // Another app's (another key's) leftovers stay.
    let other = maker::maker_parent(&case.temp, "fedcba9876543210").join(RUN_ID);
    fs::create_dir_all(&other).unwrap();
    assert_eq!(
        maker::remove_leftovers(&case.temp, KEY, Duration::from_secs(3)),
        1
    );
    assert!(!root.exists());
    assert!(other.exists());
    child.wait().unwrap();
    assert!(!alive(&child.id().to_string()));
}

/// Made again: the skill's own `xtrace-rubric.md` entry is left out of the
/// copy whatever it is (a link out of the folder, a dangling link), and the
/// refused rubric that applies is placed there read-only only when it is a
/// regular file.
#[test]
fn a_remake_leaves_out_the_skills_rubric_entry_and_places_the_refused_one() {
    use std::os::unix::fs::symlink;
    let case = Case::new("write(GOOD)");
    let own = case.target.join(RUBRIC_FILE);
    let outside = case.root.join("outside.md");
    fs::write(&outside, "no header").unwrap();
    for (link_to, name) in [
        (&outside, "outside"),
        (&case.root.join("gone.md"), "dangling"),
    ] {
        symlink(link_to, &own).unwrap();
        // A first making would refuse the link.
        assert_eq!(
            MakerStaging::create(&case.temp, KEY, RUN_ID, &case.target, &helper_dir(), None)
                .map(|_| ()),
            Err(GradeError::SkillLinkRefused),
            "{name}"
        );
        let staging = MakerStaging::create(
            &case.temp,
            KEY,
            RUN_ID,
            &case.target,
            &helper_dir(),
            Some(&own),
        )
        .unwrap();
        // Not followed, so not placed: the copy has no rubric entry.
        assert!(!staging.existing_in_target(), "{name}");
        assert!(fs::symlink_metadata(staging.target_dir().join(RUBRIC_FILE)).is_err());
        assert!(staging.target_dir().join("SKILL.md").is_file());
        // The run says the old rubric is not in TARGET.
        let written = case.run_in(
            &staging,
            Some(RubricError::NotRegularFile),
            quick(Duration::from_secs(60)),
            None,
        );
        assert!(written.unwrap().made().is_some());
        assert!(staging.remove());
        fs::remove_file(&own).unwrap();
    }
    // A broken rubric XTrace keeps elsewhere is placed as a read-only copy.
    let kept = case.root.join("kept.md");
    fs::write(&kept, "no header").unwrap();
    let staging = MakerStaging::create(
        &case.temp,
        KEY,
        RUN_ID,
        &case.target,
        &helper_dir(),
        Some(&kept),
    )
    .unwrap();
    assert!(staging.existing_in_target());
    let placed = staging.target_dir().join(RUBRIC_FILE);
    assert_eq!(fs::read_to_string(&placed).unwrap(), "no header");
    assert_eq!(
        fs::symlink_metadata(&placed).unwrap().permissions().mode() & 0o777,
        0o400
    );
    case.run_in(
        &staging,
        Some(RubricError::NoHeader),
        quick(Duration::from_secs(60)),
        None,
    )
    .unwrap();
    let records = case.records();
    assert!(Case::argv(records.last().unwrap())[2].contains(
        "The skill's current xtrace-rubric.md (a copy is in TARGET) is refused by XTrace"
    ));
    // A regular broken rubric in the folder itself is placed the same way.
    assert!(staging.remove());
    fs::write(&own, "folder copy").unwrap();
    let staging = MakerStaging::create(
        &case.temp,
        KEY,
        RUN_ID,
        &case.target,
        &helper_dir(),
        Some(&own),
    )
    .unwrap();
    assert!(staging.existing_in_target());
    assert_eq!(
        fs::read_to_string(staging.target_dir().join(RUBRIC_FILE)).unwrap(),
        "folder copy"
    );
}
