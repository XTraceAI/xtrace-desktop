//! One rubric-making run: Claude Code reads an installed skill with XTrace's
//! bundled helper skill, `xtrace-rubric-maker` ([`Helper`]), and writes one
//! file, `xtrace-rubric.md`, which XTrace then checks with the rubric reader
//! ([`crate::rubric`]). Saving the checked rubric is the caller's
//! ([`crate::rubric_store::save`]); the run itself writes only into its own
//! private staging folder.
//!
//! The run is contained exactly like a grading run ([`crate::runner`]):
//!
//! * **Where.** A private, whitespace-free staging folder per run
//!   ([`MakerStaging`]): `<tmp>/xtrace-grading/maker-<key>/<run id>/`, where
//!   `key` names the app's data folder ([`data_key`]) so leftovers of this
//!   app's runs, and only those, are found again after a crash
//!   ([`remove_leftovers`]). It holds `target/`, a copy of the skill (within
//!   [`CopyLimits::INSTALLED`], links only to files inside it); `helper/`, a
//!   copy of the helper; and `out/` (then `out-2/` for the retry), the only
//!   writable folder.
//! * **How.** The same command line as a grading run
//!   ([`runner::command_line`]): the copy of the skill is the read-only
//!   working folder (`denyWrite`, and `Edit` refused there), the output
//!   folder the only writable one (`allowWrite`, and `Edit` allowed there),
//!   the helper's copy the skill folder; no settings file is loaded, hooks,
//!   MCP servers and the network are off, the secret locations are
//!   unreadable ([`runner::SECRET_LOCATIONS`]), the transcript is not saved, and
//!   Claude Code's sandbox is required. The same minimum Claude Code version
//!   is checked first, and the child runs in its own process group with the
//!   same allow-listed environment, a deadline ([`DEFAULT_DEADLINE`]),
//!   cancel, and its process recorded in the staging folder
//!   ([`MakerStaging::leader_record`]).
//! * **What.** The prompt ([`prompt`]) is an XTrace header naming `TARGET`
//!   (the copy) and `OUT`, then the helper's own `SKILL.md`. When a rubric
//!   that XTrace refused is made again, the copy leaves out the skill's own
//!   `xtrace-rubric.md` entry (whatever it is), and the refused rubric that
//!   applies (the folder's or one XTrace keeps) is placed there instead as a
//!   read-only copy when it is a regular file; the header says why it was
//!   refused, and whether it is in `TARGET`.
//! * **Refused, or checked once more if needed.** The run writes either
//!   `OUT/xtrace-rubric.md` or, when the helper decides the skill cannot be
//!   used to score projects, `OUT/xtrace-refusal.md` with a short reason
//!   ([`read_refusal`]). A readable refusal ends the run with that reason
//!   ([`Written::Refused`]), with no retry. Otherwise the rubric is read with
//!   the rubric reader. When it is missing or refused, the refusal cannot be
//!   read, or both files were written ([`TryProblem`]), a second run gets
//!   that reason in its prompt (and, for a rubric made again, still why the
//!   old one was refused); when that one fails too, the run fails with the
//!   second reason. A run that fails on its own (a timeout, Claude Code
//!   exiting with an error) is not retried. Anything else the run wrote in
//!   its output folder is ignored.
//!
//! Only Unix hosts are supported.

use crate::{
    rubric::{self, MAX_ID_CHARS, MAX_RUBRIC_BYTES, RUBRIC_FILE, Rubric, RubricError},
    runner::{self, GradeError, Launch, Limits},
    skill::{INSTRUCTIONS_FILE, MAX_INSTRUCTIONS_BYTES, SKILL_DIR_PLACEHOLDER},
    staging::{self, CopyLimits},
};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    ffi::OsString,
    path::{Path, PathBuf},
    time::Duration,
};
use xt_probes::{leader::GroupStop, process::CancelFlag, regular_file};

/// The helper skill's name, and its folder's name among the bundled skills.
pub const HELPER_ID: &str = "xtrace-rubric-maker";
/// How long one rubric-making run may take.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(10 * 60);
/// How many runs one rubric gets: the first, and one retry with the reader's
/// reason.
pub const TRIES: usize = 2;
/// The staged copy of the skill a rubric is made for.
pub const STAGED_TARGET_DIR: &str = "target";
/// The staged copy of the helper.
pub const STAGED_HELPER_DIR: &str = "helper";
/// The output folder of each try, in order.
pub const OUT_DIRS: [&str; TRIES] = ["out", "out-2"];
/// The start of this app's rubric-making staging parent's name.
pub const PARENT_PREFIX: &str = "maker-";
/// Hexadecimal digits of [`data_key`].
const DATA_KEY_CHARS: usize = 16;
/// The highest number a clashing id is suffixed with ([`free_id`]).
const MAX_ID_SUFFIX: u32 = 99;
/// The file a run writes instead of a rubric when the skill cannot be used
/// to score projects.
pub const REFUSAL_FILE: &str = "xtrace-refusal.md";
/// Most characters of a refusal's reason kept; a longer one is cut.
pub const MAX_REFUSAL_CHARS: usize = 300;
/// Largest refusal file read.
pub const MAX_REFUSAL_BYTES: u64 = 4 * 1024;

/// What was wrong with one try's output.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TryProblem {
    /// What the rubric reader said of its rubric (missing when it wrote
    /// neither file).
    #[error("{0}")]
    Rubric(RubricError),
    /// Its refusal is not a short UTF-8 text file ([`read_refusal`]).
    #[error("the refusal could not be read as a short plain-text reason")]
    Refusal,
    /// It wrote both a rubric and a refusal.
    #[error("both a rubric and a refusal were written")]
    Both,
}

/// Why no rubric was made. No path in messages.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MakeError {
    #[error("the bundled rubric maker could not be read")]
    HelperUnavailable,
    #[error("{0}")]
    Run(GradeError),
    /// What the rubric reader said of the last try's file.
    #[error("{0}")]
    Rubric(RubricError),
    /// The last try's refusal could not be read.
    #[error("the refusal could not be read as a short plain-text reason")]
    Refusal,
    /// The last try wrote both a rubric and a refusal.
    #[error("both a rubric and a refusal were written")]
    Both,
}

impl From<TryProblem> for MakeError {
    fn from(problem: TryProblem) -> Self {
        match problem {
            TryProblem::Rubric(error) => Self::Rubric(error),
            TryProblem::Refusal => Self::Refusal,
            TryProblem::Both => Self::Both,
        }
    }
}

/// The bundled helper skill: its folder and its `SKILL.md`.
#[derive(Clone, Debug)]
pub struct Helper {
    dir: PathBuf,
    instructions: String,
}

impl Helper {
    /// Open the helper's folder: a real folder with a UTF-8 `SKILL.md` of at
    /// most [`MAX_INSTRUCTIONS_BYTES`], read as a regular file.
    pub fn open(dir: &Path) -> Result<Self, MakeError> {
        let dir = std::fs::canonicalize(dir).map_err(|_| MakeError::HelperUnavailable)?;
        if !dir.is_dir() || dir.to_str().is_none() {
            return Err(MakeError::HelperUnavailable);
        }
        let instructions =
            regular_file::read_capped(&dir.join(INSTRUCTIONS_FILE), MAX_INSTRUCTIONS_BYTES)
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .ok_or(MakeError::HelperUnavailable)?;
        Ok(Self { dir, instructions })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn instructions(&self) -> &str {
        &self.instructions
    }
}

/// The skill's current rubric, which XTrace refused, when it is made again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Existing {
    /// Why XTrace refused it.
    pub error: RubricError,
    /// Whether a copy of it is `TARGET/xtrace-rubric.md`
    /// ([`MakerStaging::existing_in_target`]).
    pub in_target: bool,
}

/// The prompt of one try: the XTrace header (what `TARGET` and `OUT` are,
/// that `TARGET` is read-only and only `OUT/xtrace-rubric.md` or
/// `OUT/xtrace-refusal.md` may be written), why the skill's current rubric
/// is refused when it is made again, what was wrong with the previous try
/// when there was one, then the helper's instructions with
/// [`SKILL_DIR_PLACEHOLDER`] replaced by its folder.
pub fn prompt(
    helper: &Helper,
    target: &str,
    out: &str,
    existing: Option<Existing>,
    previous: Option<TryProblem>,
) -> String {
    let dir = helper.dir.to_str().unwrap_or_default();
    let instructions = helper.instructions.replace(SKILL_DIR_PLACEHOLDER, dir);
    let existing = match existing {
        None
        | Some(Existing {
            error: RubricError::Missing,
            ..
        }) => String::new(),
        Some(Existing {
            error,
            in_target: true,
        }) => format!(
            "The skill's current {RUBRIC_FILE} (a copy is in TARGET) is refused by XTrace: \
             {error}. Write a new one from the skill itself and do not repeat that mistake.\n"
        ),
        Some(Existing {
            error,
            in_target: false,
        }) => format!(
            "The skill's current {RUBRIC_FILE} is refused by XTrace: {error}. It is not in \
             TARGET. Write a new one from the skill itself and do not repeat that mistake.\n"
        ),
    };
    let previous = match previous {
        None => String::new(),
        Some(TryProblem::Rubric(RubricError::Missing)) => format!(
            "A previous try of this run wrote neither {out}/{RUBRIC_FILE} nor \
             {out}/{REFUSAL_FILE}. This is the last try: write one of them, following the \
             format exactly.\n"
        ),
        Some(TryProblem::Rubric(error)) => format!(
            "A previous try of this run wrote a rubric XTrace refused: {error}. This is the last \
             try: write {out}/{RUBRIC_FILE} again, following the format exactly.\n"
        ),
        Some(TryProblem::Refusal) => format!(
            "A previous try of this run wrote a {REFUSAL_FILE} XTrace could not read: it must be \
             a UTF-8 text file holding one short plain reason of at most {MAX_REFUSAL_CHARS} \
             characters. This is the last try.\n"
        ),
        Some(TryProblem::Both) => format!(
            "A previous try of this run wrote both {RUBRIC_FILE} and {REFUSAL_FILE}. This is the \
             last try: write exactly one of them.\n"
        ),
    };
    format!(
        "XTrace rubric-making run (headless). TARGET = {target}. OUT = {out}.\n\
         TARGET is a copy of the skill to write a rubric for, and it is read-only. Write exactly \
         one file: {out}/{RUBRIC_FILE}, or {out}/{REFUSAL_FILE} when the helper instructions say \
         to refuse. Write nothing else anywhere. Do not run the skill. Nobody can answer \
         questions during this run. Follow the helper instructions below from their first step \
         to their last.\n\
         {existing}{previous}\n\
         ===== Helper instructions =====\n\n{instructions}\n\n\
         ===== End of helper instructions ====="
    )
}

/// The reason in a refusal file: a regular file (never followed through a
/// link) of at most [`MAX_REFUSAL_BYTES`], UTF-8 text without control
/// characters other than line breaks and tabs, not blank. Whitespace runs
/// become one space, and a reason longer than [`MAX_REFUSAL_CHARS`] is cut
/// there with `…`. None when it cannot be read so.
pub fn read_refusal(path: &Path) -> Option<String> {
    let bytes = regular_file::read_capped(path, MAX_REFUSAL_BYTES).ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return None;
    }
    let reason = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if reason.is_empty() {
        return None;
    }
    if reason.chars().count() <= MAX_REFUSAL_CHARS {
        return Some(reason);
    }
    let cut: String = reason.chars().take(MAX_REFUSAL_CHARS - 1).collect();
    Some(format!("{}…", cut.trim_end()))
}

/// A short, stable name for an app's data folder, so each app's
/// rubric-making staging parent is its own.
pub fn data_key(data_folder: &Path) -> String {
    Sha256::digest(data_folder.as_os_str().as_encoded_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .chars()
        .take(DATA_KEY_CHARS)
        .collect()
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

/// The parent of an app's rubric-making staging folders:
/// `<tmp>/xtrace-grading/maker-<key>`.
pub fn maker_parent(temp_dir: &Path, key: &str) -> PathBuf {
    staging::staging_parent(temp_dir).join(format!("{PARENT_PREFIX}{key}"))
}

/// One rubric-making run's staging folder. Removed when dropped, unless
/// [`Self::remove`] already did.
#[derive(Debug)]
pub struct MakerStaging {
    root: PathBuf,
    removed: bool,
    existing_in_target: bool,
}

impl MakerStaging {
    /// Create `<maker parent>/<run_id>/` with a copy of the skill at
    /// `target` (within [`CopyLimits::INSTALLED`]) and of the helper at
    /// `helper` (within [`CopyLimits::BUILT_IN`]). Both parents are made
    /// owner-only if missing and must be real folders this user owns that
    /// only it can open; their paths may not contain whitespace. The run
    /// folder must not exist yet.
    ///
    /// `existing` is the refused rubric that applies to the skill when its
    /// rubric is made again: the copy then leaves out the skill's own
    /// `xtrace-rubric.md` entry, whatever kind of entry it is (it is being
    /// replaced), and `existing` is placed there instead, read-only, when it
    /// is a regular file (never followed through a link) of at most
    /// [`MAX_RUBRIC_BYTES`] ([`Self::existing_in_target`]).
    #[cfg(unix)]
    pub fn create(
        temp_dir: &Path,
        key: &str,
        run_id: &str,
        target: &Path,
        helper: &Path,
        existing: Option<&Path>,
    ) -> Result<Self, GradeError> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if !staging::is_run_id(run_id) || !valid_key(key) {
            return Err(GradeError::StagingFailed);
        }
        staging::make_private_parent(temp_dir)?;
        let parent = staging::make_private_folder(&maker_parent(temp_dir, key))?;
        let root = parent.join(run_id);
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&root)
            .map_err(|_| GradeError::StagingFailed)?;
        let mut staging = Self {
            root,
            removed: false,
            existing_in_target: false,
        };
        let target = std::fs::canonicalize(target).map_err(|_| GradeError::StagingFailed)?;
        staging::copy_skill_except(
            &target,
            &staging.target_dir(),
            CopyLimits::INSTALLED,
            existing.map(|_| RUBRIC_FILE),
        )?;
        if let Some(bytes) =
            existing.and_then(|path| regular_file::read_capped(path, MAX_RUBRIC_BYTES).ok())
        {
            let placed = staging.target_dir().join(RUBRIC_FILE);
            staging::write_private(&placed, &bytes).map_err(|_| GradeError::StagingFailed)?;
            std::fs::set_permissions(&placed, std::fs::Permissions::from_mode(0o400))
                .map_err(|_| GradeError::StagingFailed)?;
            staging.existing_in_target = true;
        }
        let helper = std::fs::canonicalize(helper).map_err(|_| GradeError::StagingFailed)?;
        staging::copy_skill(&helper, &staging.helper_dir(), CopyLimits::BUILT_IN)?;
        Ok(staging)
    }

    #[cfg(not(unix))]
    pub fn create(
        _: &Path,
        _: &str,
        _: &str,
        _: &Path,
        _: &Path,
        _: Option<&Path>,
    ) -> Result<Self, GradeError> {
        Err(GradeError::UnsupportedPlatform)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether the refused rubric being replaced was placed in the copy as
    /// its `xtrace-rubric.md`.
    pub fn existing_in_target(&self) -> bool {
        self.existing_in_target
    }

    /// The staged copy of the skill a rubric is made for.
    pub fn target_dir(&self) -> PathBuf {
        self.root.join(STAGED_TARGET_DIR)
    }

    /// The staged copy of the helper.
    pub fn helper_dir(&self) -> PathBuf {
        self.root.join(STAGED_HELPER_DIR)
    }

    /// The output folder of try `index` (from 0); made by the run.
    pub fn out_dir(&self, index: usize) -> PathBuf {
        self.root.join(OUT_DIRS[index.min(TRIES - 1)])
    }

    /// Where a try's Claude Code process is recorded while it runs.
    pub fn leader_record(&self) -> PathBuf {
        self.root.join(staging::LEADER_RECORD)
    }

    /// Remove the staging folder now. Returns whether it is gone.
    pub fn remove(mut self) -> bool {
        self.removed = true;
        remove_tree(&self.root)
    }
}

impl Drop for MakerStaging {
    fn drop(&mut self) {
        if !self.removed {
            remove_tree(&self.root);
        }
    }
}

fn remove_tree(path: &Path) -> bool {
    match std::fs::remove_dir_all(path) {
        Ok(()) => true,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

/// After a start of the app: stop the Claude Code process each leftover
/// rubric-making staging folder of this app (`key`) recorded, only while it
/// is still exactly the recorded process ([`staging::stop_recorded_leader`],
/// waiting at most `grace`), then remove those folders. Nothing is listed or
/// removed under a parent that is a link, not this user's, or open to
/// others. Returns how many folders were removed.
pub fn remove_leftovers(temp_dir: &Path, key: &str, grace: Duration) -> usize {
    if !valid_key(key) || staging::private_parent(temp_dir).is_none() {
        return 0;
    }
    let parent = maker_parent(temp_dir, key);
    if !staging::is_private_folder(&parent) {
        return 0;
    }
    staging::remove_run_folders(&parent, |name| {
        let _: GroupStop = staging::stop_recorded_leader(&parent.join(name), grace);
        true
    })
}

/// One rubric-making run's inputs.
#[derive(Debug)]
pub struct MakeRun<'a> {
    /// The resolved absolute Claude Code executable.
    pub claude: &'a Path,
    pub staging: &'a MakerStaging,
    /// The environment this process received; only the runner's allow-list
    /// reaches the child.
    pub inherited_env: &'a [(OsString, OsString)],
    pub limits: Limits,
    /// Why the skill's current rubric is refused, when it is made again.
    pub existing: Option<RubricError>,
}

/// A rubric the run wrote that the reader accepts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Made {
    /// The file's exact bytes.
    pub bytes: Vec<u8>,
    pub rubric: Rubric,
    /// How many tries it took (1 or 2).
    pub tries: usize,
}

/// A refusal the run wrote instead of a rubric.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    /// The reason, as [`read_refusal`] reads it.
    pub reason: String,
    /// Which try wrote it (1 or 2).
    pub tries: usize,
}

/// What a run that ended well wrote.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Written {
    Rubric(Made),
    /// The skill cannot be used to score projects, for this reason.
    Refused(Refusal),
}

impl Written {
    /// The made rubric; none for a refusal.
    pub fn made(self) -> Option<Made> {
        match self {
            Self::Rubric(made) => Some(made),
            Self::Refused(_) => None,
        }
    }
}

/// Make a rubric: up to [`TRIES`] runs, as the module says.
pub fn run(spec: &MakeRun<'_>, cancel: Option<&CancelFlag>) -> Result<Written, MakeError> {
    if !cfg!(unix) {
        return Err(MakeError::Run(GradeError::UnsupportedPlatform));
    }
    if !spec.claude.is_absolute() {
        return Err(MakeError::Run(GradeError::ClaudeNotAbsolute));
    }
    let helper = Helper::open(&spec.staging.helper_dir())?;
    let helper_dir = helper.dir().to_str().ok_or(MakeError::HelperUnavailable)?;
    if runner::has_whitespace(helper_dir) {
        return Err(MakeError::Run(GradeError::SkillPathHasWhitespace));
    }
    let target = runner::resolve_project(&spec.staging.target_dir())
        .map_err(|_| MakeError::Run(GradeError::StagingFailed))?;
    let target_text = target
        .to_str()
        .ok_or(MakeError::Run(GradeError::PathNotUtf8))?;
    let record = spec.staging.leader_record();
    let existing = spec.existing.map(|error| Existing {
        error,
        in_target: spec.staging.existing_in_target(),
    });
    let mut previous = None;
    let mut last = TryProblem::Rubric(RubricError::Missing);
    for index in 0..TRIES {
        if cancel.is_some_and(CancelFlag::is_cancelled) {
            return Err(MakeError::Run(GradeError::Cancelled));
        }
        let out = make_out(&spec.staging.out_dir(index)).map_err(MakeError::Run)?;
        runner::check_pair(&target, &out).map_err(MakeError::Run)?;
        let out_text = out
            .to_str()
            .ok_or(MakeError::Run(GradeError::PathNotUtf8))?;
        let launch = Launch {
            claude: spec.claude,
            argv: runner::command_line(
                spec.claude,
                prompt(&helper, target_text, out_text, existing, previous),
                target_text,
                out_text,
                helper_dir,
            ),
            cwd: &target,
            grade_dir: &out,
            inherited_env: spec.inherited_env,
            limits: spec.limits,
            leader_record: Some(&record),
        };
        if index == 0 {
            runner::check_claude_version(&launch, cancel).map_err(MakeError::Run)?;
        }
        let ran = runner::spawn_and_wait(&launch, cancel);
        // The try's process group is reaped: its record names nothing now,
        // and the next try records its own.
        let _ = std::fs::remove_file(&record);
        ran.map_err(MakeError::Run)?;
        let tries = index + 1;
        let exists = |name: &str| std::fs::symlink_metadata(out.join(name)).is_ok();
        let problem = match (exists(RUBRIC_FILE), exists(REFUSAL_FILE)) {
            (true, true) => TryProblem::Both,
            (false, true) => match read_refusal(&out.join(REFUSAL_FILE)) {
                Some(reason) => return Ok(Written::Refused(Refusal { reason, tries })),
                None => TryProblem::Refusal,
            },
            _ => match rubric::read_file_bytes(&out.join(RUBRIC_FILE)) {
                Ok((bytes, rubric)) => {
                    return Ok(Written::Rubric(Made {
                        bytes,
                        rubric,
                        tries,
                    }));
                }
                Err(error) => TryProblem::Rubric(error),
            },
        };
        last = problem;
        previous = Some(problem);
    }
    Err(last.into())
}

/// The refused rubric of a skill whose rubric is made again.
#[derive(Clone, Copy, Debug)]
pub struct Replaced<'a> {
    /// Why the rubric reader refused it.
    pub error: RubricError,
    /// The file that applies to the skill now (its folder's own, or one
    /// XTrace keeps: [`crate::rubric_store::applicable`]); it may be a link
    /// or missing.
    pub path: &'a Path,
}

/// One whole rubric-making job, as the app runs it and `xtrace-core
/// make-rubric` runs it too.
#[derive(Debug)]
pub struct Job<'a> {
    /// The resolved absolute Claude Code executable.
    pub claude: &'a Path,
    /// The system temporary folder; staging goes in
    /// `xtrace-grading/maker-<key>/<run_id>/` there ([`MakerStaging`]).
    pub temp_dir: &'a Path,
    pub key: &'a str,
    pub run_id: &'a str,
    /// The skill's real folder.
    pub skill: &'a Path,
    /// The bundled helper's folder ([`Helper`]).
    pub helper: &'a Path,
    /// Set when the skill's rubric is made again.
    pub replaced: Option<Replaced<'a>>,
    /// The environment this process received.
    pub inherited_env: &'a [(OsString, OsString)],
    pub limits: Limits,
}

/// Run one job: stage the skill (for a remake, with the refused rubric in
/// place of its own entry: [`MakerStaging::create`]), [`run`] up to
/// [`TRIES`] tries, remove the staging folder, and give a made rubric the
/// version XTrace saves it with ([`next_version`]). Nothing is saved: the
/// caller decides where the rubric goes.
pub fn make(job: &Job<'_>, cancel: Option<&CancelFlag>) -> Result<Written, MakeError> {
    let refused = job.replaced.map(|replaced| replaced.path);
    let version = next_version(refused);
    let staging = MakerStaging::create(
        job.temp_dir,
        job.key,
        job.run_id,
        job.skill,
        job.helper,
        refused,
    )
    .map_err(MakeError::Run)?;
    let written = run(
        &MakeRun {
            claude: job.claude,
            staging: &staging,
            inherited_env: job.inherited_env,
            limits: job.limits,
            existing: job.replaced.map(|replaced| replaced.error),
        },
        cancel,
    );
    staging.remove();
    match written? {
        Written::Rubric(made) => {
            let (bytes, rubric) =
                rubric::with_version(&made.bytes, &version).map_err(MakeError::Rubric)?;
            Ok(Written::Rubric(Made {
                bytes,
                rubric,
                tries: made.tries,
            }))
        }
        refused => Ok(refused),
    }
}

/// The version XTrace gives a made rubric: `1` for a first one; for one
/// made again (`refused` names the rubric it replaces), that rubric's header
/// `version` plus one when it is a plain whole number, otherwise `2`.
pub fn next_version(refused: Option<&Path>) -> String {
    let Some(path) = refused else {
        return "1".to_owned();
    };
    regular_file::read_capped(path, MAX_RUBRIC_BYTES)
        .ok()
        .and_then(|bytes| rubric::header_value(&bytes, "version"))
        .filter(|version| !version.is_empty() && version.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|version| version.parse::<u64>().ok())
        .and_then(|version| version.checked_add(1))
        .map_or_else(|| "2".to_owned(), |next| next.to_string())
}

/// Make an empty owner-only output folder and return it resolved.
#[cfg(unix)]
fn make_out(out: &Path) -> Result<PathBuf, GradeError> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(out)
        .map_err(|_| GradeError::Prepare)?;
    std::fs::canonicalize(out).map_err(|_| GradeError::Prepare)
}

#[cfg(not(unix))]
fn make_out(_: &Path) -> Result<PathBuf, GradeError> {
    Err(GradeError::UnsupportedPlatform)
}

/// `wanted` when no skill has it (`taken`), otherwise `wanted` with the
/// smallest suffix `-2` … `-99` that is free, cut so it stays a rubric id of
/// at most [`MAX_ID_CHARS`]; none when all are taken.
pub fn free_id(wanted: &str, taken: &HashSet<String>) -> Option<String> {
    if !taken.contains(wanted) {
        return Some(wanted.to_owned());
    }
    (2..=MAX_ID_SUFFIX).find_map(|number| {
        let suffix = format!("-{number}");
        let base: String = wanted.chars().take(MAX_ID_CHARS - suffix.len()).collect();
        let candidate = format!("{}{suffix}", base.trim_end_matches('-'));
        (rubric::is_slug(&candidate) && !taken.contains(&candidate)).then_some(candidate)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clashing_id_gets_the_smallest_free_suffix_within_the_id_limit() {
        let taken: HashSet<String> = ["lint", "lint-2"].into_iter().map(String::from).collect();
        assert_eq!(free_id("readme", &taken).as_deref(), Some("readme"));
        assert_eq!(free_id("lint", &taken).as_deref(), Some("lint-3"));
        let long = "a".repeat(MAX_ID_CHARS);
        let taken: HashSet<String> = [long.clone()].into_iter().collect();
        let free = free_id(&long, &taken).unwrap();
        assert_eq!(free.len(), MAX_ID_CHARS);
        assert!(free.ends_with("-2"));
        // A cut that would end in `-` drops it.
        let dashed = format!("{}-b", "a".repeat(MAX_ID_CHARS - 3));
        let taken: HashSet<String> = [dashed.clone()].into_iter().collect();
        assert!(rubric::is_slug(&free_id(&dashed, &taken).unwrap()));
        let all: HashSet<String> = std::iter::once("x".to_owned())
            .chain((2..=MAX_ID_SUFFIX).map(|n| format!("x-{n}")))
            .collect();
        assert_eq!(free_id("x", &all), None);
    }

    #[test]
    fn data_keys_are_short_stable_and_differ_by_folder() {
        let one = data_key(Path::new(
            "/Users/you/Library/Application Support/ai.xtrace.app",
        ));
        assert_eq!(one.len(), DATA_KEY_CHARS);
        assert!(valid_key(&one));
        assert_eq!(
            one,
            data_key(Path::new(
                "/Users/you/Library/Application Support/ai.xtrace.app"
            ))
        );
        assert_ne!(one, data_key(Path::new("/tmp/test-data")));
        assert!(!valid_key("../x") && !valid_key("") && !valid_key("A"));
    }
}
