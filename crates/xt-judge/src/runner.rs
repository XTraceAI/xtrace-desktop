//! One headless grading run: Claude Code runs a grading skill on one project
//! folder, without the user's personal Claude Code settings, inside Claude
//! Code's sandbox with the project read-only and no network, and XTrace reads
//! back a validated protocol v1 result scored with the skill's rubric.
//!
//! The shape of a run is its guarantee.
//!
//! * One program, the resolved absolute `claude` the caller supplies, with
//!   exactly the argument vector of [`argv`]:
//!
//!   ```text
//!   claude -p <prompt>
//!     --tools Read,Write,Edit,Bash,Glob,Grep --permission-mode dontAsk
//!     --setting-sources ""
//!     --settings {"disableAllHooks":true,"permissions":{"deny":[<R>]},
//!       "sandbox":{"enabled":true,"failIfUnavailable":true,
//!       "allowUnsandboxedCommands":false,"filesystem":{"denyWrite":["<P>"],
//!       "allowWrite":["<G>"],"denyRead":[<D>]},
//!       "network":{"allowedDomains":[]}}}
//!     --strict-mcp-config --no-session-persistence
//!     --allowedTools Edit(/<G>/**) --disallowedTools Edit(/<P>/**)
//!     --add-dir <G> <S>
//!   ```
//!
//!   `P` is the resolved project folder, `G` the resolved grade folder and
//!   `S` the resolved skill folder, all absolute, so `Edit(/<G>/**)` reads
//!   `Edit(//Users/…/**)`: Claude Code's `//` prefix for an absolute path.
//!   `D` is [`SECRET_LOCATIONS`], well-known places that hold secrets, and
//!   `R` their `Read` deny rules ([`secret_read_rules`]): the sandbox refuses
//!   reads there by every command, and the deny rules refuse Claude Code's own
//!   file tools, which the sandbox does not cover.
//!   No settings file is loaded (no user, project or local settings, so none
//!   of the user's personal permission rules, sandbox exceptions, hooks or
//!   plugins); the settings on the command line are the only settings, and
//!   `dontAsk` refuses, without asking, every tool use these rules and the
//!   folders do not allow. The sandbox settings and rules are the ones a
//!   real Claude Code 2.1.296 was probed with (there with the user's settings
//!   file loaded as well); older versions are refused before the run
//!   ([`MIN_CLAUDE_VERSION`]). Commands run
//!   in Claude Code's sandbox, which refuses every write into `P` at the
//!   operating-system level, allows writes into `G` (and the system's
//!   per-user temporary folder, its default), and refuses the network; a
//!   command needs no allow rule of its own. The file-writing tools are
//!   allowed in `G` and refused in `P` by the two `Edit` rules (`Edit` rules
//!   cover every file-writing tool). `failIfUnavailable` makes Claude Code
//!   refuse to start without a working sandbox ([`GradeError::SandboxUnavailable`]).
//!   Hooks are off, no MCP server is loaded, and the run's own transcript is
//!   not saved, so it is neither graded by a later run nor counted in
//!   XTrace's history. `G` and `S` may not contain whitespace (checked
//!   before anything runs): Claude Code's allow rules were seen not to match
//!   such paths.
//! * The prompt ([`GradingSkill::prompt`]) is the skill's own `SKILL.md`,
//!   unchanged, between an XTrace header (the project is read-only; work
//!   goes in `G`) and a final XTrace step: judge the skill's output against
//!   its rubric and write `<G>/result.json`.
//! * Before the run, `claude --version` runs once, bounded and cancellable, in the
//!   empty grade folder with the same environment; a version older than
//!   [`MIN_CLAUDE_VERSION`], or no readable version, refuses the run
//!   ([`GradeError::ClaudeTooOld`]).
//! * The child runs in the project folder with no standard input, in its own
//!   process group, with an allow-listed environment ([`child_env`]): `HOME`,
//!   `USER`, `LOGNAME`, `LANG`, `LC_*`, `TMPDIR`, a `PATH` of absolute
//!   entries, and the run's own `XTRACE_GRADE_DIR`, `DISABLE_AUTOUPDATER=1`
//!   and `PYTHONDONTWRITEBYTECODE=1`. Every other variable, including
//!   `ANTHROPIC_*`, `CLAUDECODE` and `CLAUDE_CODE_*`, is dropped.
//! * With [`GradeRun::leader_record`], the child's process ID and start time
//!   are written there as soon as it starts, before the run is awaited.
//! * Both output streams are drained and capped and never reported; standard
//!   error is only looked at for Claude Code's sandbox refusal
//!   ([`SANDBOX_REFUSALS`]). A deadline ([`DEFAULT_DEADLINE`]) or a cancel
//!   ends the whole group and reaps it ([`xt_probes::process`]): `SIGTERM`
//!   first, then `SIGKILL` after [`DEFAULT_TERM_GRACE`], so a member that
//!   ignores the request still goes.
//! * The project's `.claude-md-doctor` entry (the bundled skill's default
//!   work folder) is looked at before the child starts and again after it
//!   exits: its names, kinds, sizes and modification times, at most
//!   [`MAX_SNAPSHOT_ENTRIES`] entries ([`project_entry_snapshot`]). Any
//!   difference, including the entry appearing, fails the run; one that is a
//!   link or too large to look at refuses the run before anything starts.
//!   Nothing in the project is deleted either way. The sandbox and the
//!   `Edit` deny rule refuse such writes first; this check stays because the
//!   probe proved the file-writing tools' refusal only for a project path
//!   without whitespace.
//! * Then `<G>/result.json` is read and validated ([`crate::protocol`]), and
//!   its criteria must be exactly the rubric's ([`protocol::check_rubric`]).
//!
//! Only Unix hosts are supported.

use crate::{
    protocol::{self, GradeResult, ResultError},
    skill::GradingSkill,
};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Duration,
};
pub use xt_probes::executable::Unavailable;
use xt_probes::process::CancelFlag;

/// How long one run may take.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(45 * 60);
/// Retained stdout; more is drained and dropped.
pub const DEFAULT_MAX_STDOUT_BYTES: usize = 1024 * 1024;
/// Retained stderr; more is drained and dropped.
pub const DEFAULT_MAX_STDERR_BYTES: usize = 256 * 1024;
/// After `claude` exits, how long a leftover descendant may hold its pipes
/// before the group is ended.
pub const DEFAULT_EXIT_GRACE: Duration = Duration::from_secs(10);
/// When a deadline, a cancel or a leftover group ends a run, how long the
/// group has after `SIGTERM` before `SIGKILL`.
pub const DEFAULT_TERM_GRACE: Duration = Duration::from_secs(3);
/// Most entries the before/after look at a project's
/// [`PROJECT_WORK_ENTRY`] covers; a larger one refuses the run.
pub const MAX_SNAPSHOT_ENTRIES: usize = 500;
/// Deepest folder level that look descends to; deeper refuses the run.
const MAX_SNAPSHOT_DEPTH: usize = 16;
/// The work folder's name inside the grade folder.
pub const WORK_DIR: &str = "work";
/// The bundled skill's default work folder inside an examined project. A run
/// that creates it wrote into the user's project.
pub const PROJECT_WORK_ENTRY: &str = ".claude-md-doctor";
/// The variable naming the run's grade folder to the skill's scripts.
pub const GRADE_DIR_ENV: &str = "XTRACE_GRADE_DIR";
/// The tools Claude Code may use at all.
pub const TOOLS: &str = "Read,Write,Edit,Bash,Glob,Grep";
/// The settings files Claude Code loads: none. The user's personal settings
/// (permission rules, sandbox exceptions, hooks, plugins) never apply to a
/// grading run; the settings on the command line are the only settings.
pub const SETTING_SOURCES: &str = "";
/// The oldest Claude Code a run starts: the version whose sandbox, settings
/// and permission behaviour were probed. An older one, or one whose version
/// cannot be read, is [`GradeError::ClaudeTooOld`].
pub const MIN_CLAUDE_VERSION: (u32, u32, u32) = (2, 1, 296);
/// How long `claude --version` may take.
pub const DEFAULT_VERSION_DEADLINE: Duration = Duration::from_secs(30);
/// Retained `claude --version` output; more is dropped.
const MAX_VERSION_OUTPUT_BYTES: usize = 4096;
/// What Claude Code writes to standard error when it refuses to start
/// because the sandbox is required but unavailable (Claude Code 2.1.296:
/// "Error: sandbox required but unavailable: …" then "sandbox.failIfUnavailable
/// is set — refusing to start without a working sandbox."). Either one,
/// with a failed exit, is [`GradeError::SandboxUnavailable`].
pub const SANDBOX_REFUSALS: [&str; 2] = [
    "sandbox required but unavailable",
    "sandbox.failIfUnavailable is set",
];
/// Inherited variables kept by name; `LC_*` is kept by prefix.
const KEPT_ENV: [&str; 5] = ["HOME", "USER", "LOGNAME", "LANG", "TMPDIR"];

/// The bounds of one run. Tests shorten them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    pub deadline: Duration,
    pub max_stdout_bytes: usize,
    pub max_stderr_bytes: usize,
    pub exit_grace: Duration,
    pub term_grace: Duration,
    /// How long the `claude --version` check before the run may take.
    pub version_deadline: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            deadline: DEFAULT_DEADLINE,
            max_stdout_bytes: DEFAULT_MAX_STDOUT_BYTES,
            max_stderr_bytes: DEFAULT_MAX_STDERR_BYTES,
            exit_grace: DEFAULT_EXIT_GRACE,
            term_grace: DEFAULT_TERM_GRACE,
            version_deadline: DEFAULT_VERSION_DEADLINE,
        }
    }
}

/// Why a run produced no result. Messages hold no path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GradeError {
    #[error("grading needs a Unix host")]
    UnsupportedPlatform,
    #[error("the Claude Code executable must be an absolute path")]
    ClaudeNotAbsolute,
    #[error("the project folder does not exist or is not a folder")]
    ProjectInvalid,
    #[error("the grade folder does not exist or is not a folder")]
    GradeDirInvalid,
    #[error("the grade folder must be empty")]
    GradeDirNotEmpty,
    #[error("the grade folder and the project folder must not contain each other")]
    GradeDirOverlapsProject,
    #[error("a folder path is not valid UTF-8")]
    PathNotUtf8,
    #[error("the grading skill folder's path contains whitespace")]
    SkillPathHasWhitespace,
    #[error("the grade folder's path contains whitespace")]
    GradeDirPathHasWhitespace,
    #[error("the grade folder could not be prepared")]
    Prepare,
    #[error("the grading run's private folders could not be prepared")]
    StagingFailed,
    #[error("the grading skill's folder has more files, bytes or folder levels than are copied")]
    SkillTooLarge,
    #[error("the grading skill's folder has a link to a folder, out of it, or to nothing")]
    SkillLinkRefused,
    #[error("the grading result could not be kept")]
    ArchiveFailed,
    #[error("Claude Code could not be started")]
    Spawn,
    #[error("the grading run was cancelled")]
    Cancelled,
    #[error("the grading run took longer than its deadline")]
    Timeout,
    #[error("the grading run could not be observed")]
    Unobservable,
    #[error("Claude Code exited with a failure")]
    Exited { code: Option<i32> },
    #[error("the grading run wrote into the project folder")]
    ProjectWritten,
    #[error(
        "a .claude-md-doctor folder in the project is a link, too large or unreadable to check"
    )]
    ProjectWorkFolderUncheckable,
    #[error("Claude Code's sandbox is not available on this computer")]
    SandboxUnavailable,
    #[error("Claude Code is older than grading needs, or its version could not be read")]
    ClaudeTooOld,
    #[error("{0}")]
    Result(#[from] ResultError),
}

/// One run's inputs.
#[derive(Clone, Debug)]
pub struct GradeRun<'a> {
    /// The resolved absolute Claude Code executable.
    pub claude: &'a Path,
    pub skill: &'a GradingSkill,
    /// The project folder to examine.
    pub project: &'a Path,
    /// An existing, empty folder the run owns.
    pub grade_dir: &'a Path,
    /// The environment this process received; only the allow-list reaches
    /// the child.
    pub inherited_env: &'a [(OsString, OsString)],
    pub limits: Limits,
    /// Where to write the identity of the launched Claude Code, the leader
    /// of the run's process group ([`xt_probes::leader`]), right after it
    /// starts, so a later start of the app can stop a run that outlived it
    /// ([`crate::staging::stop_recorded_leader`]). The file must not exist
    /// yet; when it cannot be written, or the child is still going but
    /// cannot be observed, the run is ended and fails
    /// ([`GradeError::Prepare`]). `None` records nothing.
    pub leader_record: Option<&'a Path>,
}

/// The resolved folders of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folders {
    pub project: PathBuf,
    pub grade_dir: PathBuf,
    pub work: PathBuf,
}

/// One headless Claude Code launch: the exact command line, where it runs
/// and what it may record. Both a grading run and a rubric-making run
/// ([`crate::maker`]) launch through this, with the same checks: the
/// version check first ([`check_claude_version`]), then one bounded,
/// cancellable child in its own process group ([`spawn_and_wait`]).
pub(crate) struct Launch<'a> {
    pub claude: &'a Path,
    /// From [`command_line`].
    pub argv: Vec<OsString>,
    /// The read-only folder the child runs in.
    pub cwd: &'a Path,
    /// The only writable folder; the version check runs there and the
    /// child sees it as [`GRADE_DIR_ENV`].
    pub grade_dir: &'a Path,
    pub inherited_env: &'a [(OsString, OsString)],
    pub limits: Limits,
    pub leader_record: Option<&'a Path>,
}

/// Resolve and check the folders: both exist, the grade folder is empty, and
/// neither contains the other.
pub fn folders(project: &Path, grade_dir: &Path) -> Result<Folders, GradeError> {
    let project = resolve_project(project)?;
    let grade_dir = std::fs::canonicalize(grade_dir)
        .ok()
        .filter(|path| path.is_dir())
        .ok_or(GradeError::GradeDirInvalid)?;
    check_pair(&project, &grade_dir)?;
    let mut entries = std::fs::read_dir(&grade_dir).map_err(|_| GradeError::GradeDirInvalid)?;
    if entries.next().is_some() {
        return Err(GradeError::GradeDirNotEmpty);
    }
    let work = grade_dir.join(WORK_DIR);
    Ok(Folders {
        project,
        grade_dir,
        work,
    })
}

/// Check a grade folder that may not exist yet against a project, before
/// anything is created: the project exists, and the grade folder, resolved
/// through its nearest existing ancestor, is UTF-8, has no whitespace and
/// neither contains nor is inside the project. The same rules as
/// [`folders`], for a caller that creates the grade folder itself.
pub fn check_planned(project: &Path, grade_dir: &Path) -> Result<(), GradeError> {
    let project = resolve_project(project)?;
    let grade_dir = resolve_planned(grade_dir).ok_or(GradeError::GradeDirInvalid)?;
    check_pair(&project, &grade_dir)
}

pub(crate) fn resolve_project(project: &Path) -> Result<PathBuf, GradeError> {
    let project = std::fs::canonicalize(project)
        .ok()
        .filter(|path| path.is_dir())
        .ok_or(GradeError::ProjectInvalid)?;
    if project.to_str().is_none() {
        return Err(GradeError::PathNotUtf8);
    }
    Ok(project)
}

/// The one rule for a resolved project and grade folder pair.
pub(crate) fn check_pair(project: &Path, grade_dir: &Path) -> Result<(), GradeError> {
    let grade = grade_dir.to_str().ok_or(GradeError::PathNotUtf8)?;
    if has_whitespace(grade) {
        return Err(GradeError::GradeDirPathHasWhitespace);
    }
    if project.starts_with(grade_dir) || grade_dir.starts_with(project) {
        return Err(GradeError::GradeDirOverlapsProject);
    }
    Ok(())
}

/// A path that may not exist yet, resolved: its nearest existing ancestor
/// with symbolic links resolved, then the missing rest. `None` for a rest
/// that names `..` (it cannot be resolved without the folders existing) or
/// when no ancestor can be resolved.
fn resolve_planned(path: &Path) -> Option<PathBuf> {
    let path = std::path::absolute(path).ok()?;
    let mut existing = path.as_path();
    let mut missing = Vec::new();
    loop {
        match std::fs::canonicalize(existing) {
            Ok(resolved) => {
                let mut resolved = resolved;
                for part in missing.iter().rev() {
                    resolved.push(part);
                }
                return Some(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = existing.file_name()?;
                missing.push(name.to_owned());
                existing = existing.parent()?;
            }
            Err(_) => return None,
        }
    }
}

pub(crate) fn has_whitespace(path: &str) -> bool {
    path.chars().any(char::is_whitespace)
}

fn utf8(path: &Path) -> &str {
    path.to_str().unwrap_or_default()
}

/// The settings given on the command line, the only settings: hooks off,
/// and the sandbox on and required, with the project `project` read-only,
/// the grade folder `grade` writable, no network, and the
/// [`SECRET_LOCATIONS`] unreadable ([`secret_read_rules`]). Both paths are
/// resolved and UTF-8.
pub fn settings(project: &str, grade: &str) -> String {
    let (project, grade) = (
        serde_json::Value::from(project),
        serde_json::Value::from(grade),
    );
    let deny_read = serde_json::Value::from(SECRET_LOCATIONS.to_vec());
    let read_rules = serde_json::Value::from(secret_read_rules());
    format!(
        r#"{{"disableAllHooks":true,"permissions":{{"deny":{read_rules}}},"sandbox":{{"enabled":true,"failIfUnavailable":true,"allowUnsandboxedCommands":false,"filesystem":{{"denyWrite":[{project}],"allowWrite":[{grade}],"denyRead":{deny_read}}},"network":{{"allowedDomains":[]}}}}}}"#
    )
}

/// Well-known places on the user's computer that hold secrets: keys,
/// cloud and package-registry credentials, other coding tools' sign-ins,
/// keychains and browser profiles. No grading or rubric-making run may read
/// them, whatever the skill or the project's own instructions ask. Each is
/// relative to the home folder (`~/`), and covers everything inside it.
///
/// The bundled claude-md-doctor skill reads `~/.claude/projects`,
/// `~/.claude/CLAUDE.md`, `~/.claude/rules` and `~/.claude/settings.json`;
/// none of them is covered. A secret location a run would need to read is
/// a skill that cannot be graded. On macOS the sandbox also hides whether
/// anything there exists, so a project instruction that names such a path
/// (say `~/.aws/config`) reads as a missing file to the run.
///
/// Claude Code takes them twice, and both are needed: as the sandbox's
/// `denyRead`, which the operating system enforces on every command and the
/// programs it starts, and as `Read` deny rules ([`secret_read_rules`]),
/// which Claude Code's own file tools (Read, Grep, Glob) follow; the sandbox
/// does not cover those tools.
pub const SECRET_LOCATIONS: [&str; 47] = [
    // Keys and signing.
    "~/.ssh",
    "~/.gnupg",
    "~/.password-store",
    // Cloud, cluster and container credentials.
    "~/.aws",
    "~/.azure",
    "~/.config/gcloud",
    "~/.kube",
    "~/.docker",
    "~/.vault-token",
    "~/.terraform.d/credentials.tfrc.json",
    "~/.config/op",
    // Git hosting and package registries.
    "~/.config/gh",
    "~/.config/hub",
    "~/.git-credentials",
    "~/.config/git/credentials",
    "~/.netrc",
    "~/.npmrc",
    "~/.pypirc",
    "~/.cargo/credentials",
    "~/.cargo/credentials.toml",
    "~/.gem/credentials",
    // Database passwords.
    "~/.pgpass",
    "~/.my.cnf",
    // Other coding tools' sign-ins.
    "~/.claude.json",
    "~/.claude/.credentials.json",
    "~/.codex/auth.json",
    "~/.gemini",
    "~/.config/github-copilot",
    // Keychains.
    "~/Library/Keychains",
    "~/.local/share/keyrings",
    // Browser profiles, cookies and saved passwords.
    "~/Library/Cookies",
    "~/Library/Safari",
    "~/Library/Application Support/Google/Chrome",
    "~/Library/Application Support/Chromium",
    "~/Library/Application Support/BraveSoftware",
    "~/Library/Application Support/Microsoft Edge",
    "~/Library/Application Support/Arc",
    "~/Library/Application Support/Firefox",
    "~/Library/Application Support/com.operasoftware.Opera",
    "~/Library/Application Support/Vivaldi",
    "~/.mozilla",
    "~/.config/google-chrome",
    "~/.config/chromium",
    "~/.config/BraveSoftware",
    "~/.config/microsoft-edge",
    "~/.config/opera",
    "~/.config/vivaldi",
];

/// The `Read` deny rules for [`SECRET_LOCATIONS`]: for each, the location
/// itself and everything inside it (`~/.ssh` gives `Read(~/.ssh)` and
/// `Read(~/.ssh/**)`).
pub fn secret_read_rules() -> Vec<String> {
    SECRET_LOCATIONS
        .iter()
        .flat_map(|location| [format!("Read({location})"), format!("Read({location}/**)")])
        .collect()
}

/// The exact argument vector, `argv[0]` first.
pub fn argv(claude: &Path, skill: &GradingSkill, folders: &Folders) -> Vec<OsString> {
    let grade = utf8(&folders.grade_dir);
    let project = utf8(&folders.project);
    let prompt = skill.prompt(project, grade, utf8(&folders.work));
    command_line(claude, prompt, project, grade, utf8(skill.dir()))
}

/// The one Claude Code command line every headless run uses, `argv[0]`
/// first: `prompt`, with the folder `project` read-only (the run's working
/// folder), `grade` the only writable folder and `skill_dir` the skill's
/// folder, all resolved, absolute and UTF-8. A grading run passes the
/// examined project, its grade folder and the grading skill; a rubric-making
/// run ([`crate::maker`]) the staged copy of the skill it writes a rubric
/// for, its output folder and the staged helper skill.
pub fn command_line(
    claude: &Path,
    prompt: String,
    project: &str,
    grade: &str,
    skill_dir: &str,
) -> Vec<OsString> {
    [
        claude.as_os_str().to_owned(),
        "-p".into(),
        prompt.into(),
        "--tools".into(),
        TOOLS.into(),
        "--permission-mode".into(),
        "dontAsk".into(),
        "--setting-sources".into(),
        SETTING_SOURCES.into(),
        "--settings".into(),
        settings(project, grade).into(),
        "--strict-mcp-config".into(),
        "--no-session-persistence".into(),
        "--allowedTools".into(),
        format!("Edit(/{grade}/**)").into(),
        "--disallowedTools".into(),
        format!("Edit(/{project}/**)").into(),
        "--add-dir".into(),
        grade.into(),
        skill_dir.into(),
    ]
    .into_iter()
    .collect()
}

/// The child's whole environment, from the inherited one.
pub fn child_env(
    claude: &Path,
    inherited: &[(OsString, OsString)],
    grade_dir: &Path,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = inherited
        .iter()
        .filter(|(key, _)| {
            key.to_str()
                .is_some_and(|key| KEPT_ENV.contains(&key) || key.starts_with("LC_"))
        })
        .cloned()
        .collect();
    let inherited_path = inherited_var(inherited, "PATH");
    // Relative entries would resolve against the project folder, which is
    // not a place a program may come from.
    let entries: Vec<PathBuf> =
        xt_probes::executable::claude_child_path_entries(claude, inherited_path)
            .into_iter()
            .filter(|entry| entry.is_absolute())
            .collect();
    if let Some(path) = xt_probes::executable::join_path(&entries) {
        env.push(("PATH".into(), path));
    }
    env.push((GRADE_DIR_ENV.into(), grade_dir.as_os_str().to_owned()));
    env.push(("DISABLE_AUTOUPDATER".into(), "1".into()));
    // Keeps the skill's scripts from writing bytecode beside themselves.
    env.push(("PYTHONDONTWRITEBYTECODE".into(), "1".into()));
    env
}

/// Run one grading skill on one project and return its validated result.
pub fn run(spec: &GradeRun<'_>, cancel: Option<&CancelFlag>) -> Result<GradeResult, GradeError> {
    if !cfg!(unix) {
        return Err(GradeError::UnsupportedPlatform);
    }
    if !spec.claude.is_absolute() {
        return Err(GradeError::ClaudeNotAbsolute);
    }
    let skill_dir = spec.skill.dir().to_str().ok_or(GradeError::PathNotUtf8)?;
    if has_whitespace(skill_dir) {
        return Err(GradeError::SkillPathHasWhitespace);
    }
    let folders = folders(spec.project, spec.grade_dir)?;
    if cancel.is_some_and(CancelFlag::is_cancelled) {
        return Err(GradeError::Cancelled);
    }
    let entry = folders.project.join(PROJECT_WORK_ENTRY);
    let before = project_entry_snapshot(&entry).ok_or(GradeError::ProjectWorkFolderUncheckable)?;
    let launch = Launch {
        claude: spec.claude,
        argv: argv(spec.claude, spec.skill, &folders),
        cwd: &folders.project,
        grade_dir: &folders.grade_dir,
        inherited_env: spec.inherited_env,
        limits: spec.limits,
        leader_record: spec.leader_record,
    };
    check_claude_version(&launch, cancel)?;
    std::fs::create_dir(&folders.work).map_err(|_| GradeError::Prepare)?;
    let outcome = spawn_and_wait(&launch, cancel);
    if project_entry_snapshot(&entry).as_ref() != Some(&before) {
        return Err(GradeError::ProjectWritten);
    }
    outcome?;
    let result = protocol::read(&folders.grade_dir)?;
    protocol::check_rubric(&result, spec.skill.rubric())?;
    Ok(result)
}

/// The `major.minor.patch` that `claude --version` prints first
/// ("2.1.296 (Claude Code)"); `None` for anything else.
pub fn parse_claude_version(stdout: &[u8]) -> Option<(u32, u32, u32)> {
    let text = std::str::from_utf8(stdout).ok()?;
    let token = text.split_whitespace().next()?;
    let mut parts = token.split('.');
    let mut number = || -> Option<u32> {
        let part = parts.next()?;
        (!part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| part.parse().ok())
            .flatten()
    };
    let version = (number()?, number()?, number()?);
    parts.next().is_none().then_some(version)
}

/// Run `claude --version` (bounded, cancellable, in the empty grade folder,
/// with the run's environment) and refuse a version older than
/// [`MIN_CLAUDE_VERSION`], a failed exit or output that names no version.
#[cfg(unix)]
pub(crate) fn check_claude_version(
    spec: &Launch<'_>,
    cancel: Option<&CancelFlag>,
) -> Result<(), GradeError> {
    use std::process::{Command, Stdio};
    use xt_probes::process::{self, Bounds, Ended};

    let mut command = Command::new(spec.claude);
    command
        .arg("--version")
        .current_dir(spec.grade_dir)
        .env_clear()
        .envs(child_env(spec.claude, spec.inherited_env, spec.grade_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = process::spawn_grouped(&mut command).map_err(|_| GradeError::Spawn)?;
    let bounds = Bounds {
        deadline: spec.limits.version_deadline,
        max_stdout_bytes: MAX_VERSION_OUTPUT_BYTES,
        max_stderr_bytes: MAX_VERSION_OUTPUT_BYTES,
        stdout_overflow_ends: false,
        exit_grace: Some(spec.limits.exit_grace),
        term_grace: Some(spec.limits.term_grace),
    };
    let finished = process::collect(child, bounds, cancel).map_err(|ended| match ended {
        Ended::Timeout => GradeError::Timeout,
        Ended::Cancelled => GradeError::Cancelled,
        Ended::OutputTooLarge | Ended::Unobservable => GradeError::Unobservable,
    })?;
    let version = finished
        .status
        .success()
        .then(|| parse_claude_version(&finished.stdout.bytes))
        .flatten();
    match version {
        Some(version) if version >= MIN_CLAUDE_VERSION => Ok(()),
        _ => Err(GradeError::ClaudeTooOld),
    }
}

#[cfg(not(unix))]
pub(crate) fn check_claude_version(
    _: &Launch<'_>,
    _: Option<&CancelFlag>,
) -> Result<(), GradeError> {
    Err(GradeError::UnsupportedPlatform)
}

#[cfg(unix)]
pub(crate) fn spawn_and_wait(
    spec: &Launch<'_>,
    cancel: Option<&CancelFlag>,
) -> Result<(), GradeError> {
    use std::process::{Command, Stdio};
    use xt_probes::process::{self, Bounds, Ended};

    let argv = &spec.argv;
    let mut command = Command::new(&argv[0]);
    command
        .args(&argv[1..])
        .current_dir(spec.cwd)
        .env_clear()
        .envs(child_env(spec.claude, spec.inherited_env, spec.grade_dir))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = process::spawn_grouped(&mut command).map_err(|_| GradeError::Spawn)?;
    let recorded = leader_recorded(
        spec.leader_record,
        child.id(),
        xt_probes::leader::GroupLeader::observe,
        process::exit_observed,
    );
    let stopped = CancelFlag::new();
    let cancel = if recorded {
        cancel
    } else {
        // Unrecorded, the run could not be stopped after a crash of the app:
        // end it now instead.
        stopped.cancel();
        Some(&stopped)
    };
    let bounds = Bounds {
        deadline: spec.limits.deadline,
        max_stdout_bytes: spec.limits.max_stdout_bytes,
        max_stderr_bytes: spec.limits.max_stderr_bytes,
        stdout_overflow_ends: false,
        exit_grace: Some(spec.limits.exit_grace),
        term_grace: Some(spec.limits.term_grace),
    };
    let finished = process::collect(child, bounds, cancel).map_err(|ended| match ended {
        Ended::Cancelled if !recorded => GradeError::Prepare,
        Ended::Timeout => GradeError::Timeout,
        Ended::Cancelled => GradeError::Cancelled,
        Ended::OutputTooLarge | Ended::Unobservable => GradeError::Unobservable,
    })?;
    if !finished.status.success() {
        if sandbox_refused(&finished.stderr.bytes) {
            return Err(GradeError::SandboxUnavailable);
        }
        return Err(GradeError::Exited {
            code: finished.status.code(),
        });
    }
    Ok(())
}

/// Whether captured standard error holds Claude Code's refusal to start
/// without a working sandbox ([`SANDBOX_REFUSALS`]).
pub fn sandbox_refused(stderr: &[u8]) -> bool {
    let text = String::from_utf8_lossy(stderr);
    SANDBOX_REFUSALS
        .iter()
        .any(|refusal| text.contains(refusal))
}

/// Whether the launched child is recorded as [`GradeRun::leader_record`]
/// asks, so a later start of the app could stop it; `false` ends the run.
///
/// The child is unreaped, so `pid` is still this run's. Nothing to record is
/// `true`; so is a child that cannot be observed because it already exited
/// (`exited`, which never reaps it). A child still going that cannot be
/// observed, or whose exit cannot be looked at, could not be stopped after a
/// crash of the app, exactly like one whose record cannot be written:
/// `false`.
#[cfg(unix)]
fn leader_recorded(
    record: Option<&Path>,
    pid: u32,
    observe: impl FnOnce(u32) -> Option<xt_probes::leader::GroupLeader>,
    exited: impl FnOnce(u32) -> std::io::Result<bool>,
) -> bool {
    let Some(path) = record else {
        return true;
    };
    match observe(pid) {
        Some(leader) => crate::staging::write_private(path, leader.record().as_bytes()).is_ok(),
        None => matches!(exited(pid), Ok(true)),
    }
}

#[cfg(not(unix))]
pub(crate) fn spawn_and_wait(_: &Launch<'_>, _: Option<&CancelFlag>) -> Result<(), GradeError> {
    Err(GradeError::UnsupportedPlatform)
}

/// What a project's [`PROJECT_WORK_ENTRY`] looked like: absent, or every
/// entry under it (itself first), each with its kind, size and modification
/// time, never following a symbolic link.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntrySnapshot {
    Absent,
    Present(Vec<SnapshotEntry>),
}

/// One entry of an [`EntrySnapshot`], relative to the snapshotted entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub path: PathBuf,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<std::time::SystemTime>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Folder,
    File,
    Link,
    Other,
}

/// Look at `entry` and everything under it, without following links. `None`
/// when it is itself a link (what it points to is outside the project and
/// could change unseen), holds more than [`MAX_SNAPSHOT_ENTRIES`] entries,
/// is deeper than the depth bound, or cannot be read: such an entry cannot
/// be compared.
pub fn project_entry_snapshot(entry: &Path) -> Option<EntrySnapshot> {
    let meta = match std::fs::symlink_metadata(entry) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(EntrySnapshot::Absent);
        }
        Err(_) => return None,
    };
    if meta.file_type().is_symlink() {
        return None;
    }
    let mut entries = vec![snapshot_entry(PathBuf::new(), &meta)];
    let mut pending = vec![(PathBuf::new(), 0_usize)];
    if !meta.is_dir() {
        pending.clear();
    }
    while let Some((relative, depth)) = pending.pop() {
        if depth >= MAX_SNAPSHOT_DEPTH {
            return None;
        }
        let mut names: Vec<OsString> = std::fs::read_dir(entry.join(&relative))
            .ok()?
            .map(|child| child.map(|child| child.file_name()))
            .collect::<Result<_, _>>()
            .ok()?;
        names.sort();
        for name in names {
            let path = relative.join(&name);
            let meta = std::fs::symlink_metadata(entry.join(&path)).ok()?;
            if meta.is_dir() {
                pending.push((path.clone(), depth + 1));
            }
            entries.push(snapshot_entry(path, &meta));
            if entries.len() > MAX_SNAPSHOT_ENTRIES {
                return None;
            }
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Some(EntrySnapshot::Present(entries))
}

fn snapshot_entry(path: PathBuf, meta: &std::fs::Metadata) -> SnapshotEntry {
    let kind = meta.file_type();
    SnapshotEntry {
        path,
        kind: if kind.is_symlink() {
            EntryKind::Link
        } else if kind.is_dir() {
            EntryKind::Folder
        } else if kind.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        },
        size: meta.len(),
        modified: meta.modified().ok(),
    }
}

/// The Claude Code a run uses, from an environment: the `XTRACE_CLAUDE`
/// override, then `PATH`, then Claude Code's installer folders under `HOME`
/// and Homebrew's ([`xt_probes::executable::resolve_claude`]).
pub fn find_claude(env: &[(OsString, OsString)]) -> Result<PathBuf, Unavailable> {
    xt_probes::executable::resolve_claude(
        inherited_var(env, xt_probes::executable::CLAUDE_ENV),
        inherited_var(env, "PATH"),
        inherited_var(env, "HOME").map(Path::new),
    )
}

/// This process's environment, for [`GradeRun::inherited_env`].
pub fn inherited_env() -> Vec<(OsString, OsString)> {
    std::env::vars_os().collect()
}

/// The value of one inherited variable.
pub fn inherited_var<'a>(env: &'a [(OsString, OsString)], key: &str) -> Option<&'a OsStr> {
    env.iter()
        .rev()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_os_str())
}

#[cfg(all(test, unix))]
mod tests {
    use super::leader_recorded;
    use std::time::{Duration, Instant};
    use xt_probes::{leader::GroupLeader, process};

    fn scratch() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("leader-recorded")
            .tempdir_in(std::env::temp_dir())
            .unwrap()
    }

    fn sleeper() -> std::process::Child {
        let mut command = std::process::Command::new("/bin/sleep");
        command.arg("30").stdin(std::process::Stdio::null());
        process::spawn_grouped(&mut command).unwrap()
    }

    /// A child still going that cannot be observed is not recorded, so the
    /// run ends, exactly as when its record cannot be written; one that
    /// already exited has nothing to record.
    #[test]
    fn an_unobservable_child_still_going_is_not_recorded() {
        let dir = scratch();
        let path = dir.path().join("leader");
        let mut child = sleeper();
        let pid = child.id();
        assert!(!leader_recorded(
            Some(&path),
            pid,
            |_| None,
            process::exit_observed
        ));
        assert!(!path.exists(), "nothing was written");
        // An exit that cannot be looked at is no proof the child is gone.
        assert!(!leader_recorded(
            Some(&path),
            pid,
            |_| None,
            |_| Err(std::io::ErrorKind::Other.into())
        ));
        child.kill().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !process::exit_observed(pid).unwrap() {
            assert!(Instant::now() < deadline, "the child never exited");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(leader_recorded(
            Some(&path),
            pid,
            |_| None,
            process::exit_observed
        ));
        assert!(!path.exists());
        child.wait().unwrap();
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn an_observed_child_is_recorded_once_and_never_over_another_record() {
        let dir = scratch();
        let path = dir.path().join("leader");
        let mut child = sleeper();
        let pid = child.id();
        let never =
            |_| -> std::io::Result<bool> { panic!("an observed child needs no exit check") };
        assert!(leader_recorded(
            Some(&path),
            pid,
            GroupLeader::observe,
            never
        ));
        let recorded = GroupLeader::parse(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(recorded.pid, pid);
        // A second record is refused: the file is never written over.
        assert!(!leader_recorded(
            Some(&path),
            pid,
            GroupLeader::observe,
            never
        ));
        // Nothing asked for: nothing is observed or written.
        assert!(leader_recorded(
            None,
            pid,
            |_| panic!("not observed"),
            never
        ));
        child.kill().unwrap();
        child.wait().unwrap();
    }
}
