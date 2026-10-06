//! The one maintained description of the Codex CLI launch this version
//! proves — a literal fresh `codex exec --json` run — and of where its new
//! thread's identity comes from: the `thread.started` event that run prints
//! first.
//!
//! The command is read by the reviewed restricted shell reader
//! ([`super::shell`]): literal words and nothing else, no leading
//! assignment (one could move where the session is saved), standard output
//! not redirected (the events would not be in the result), and at most the
//! standard-input and standard-error redirections that reader accepts (an
//! absolute path outside `/dev`, or `2>&1`), never opened. The
//! program is the bare name `codex`; a path to a launcher is not read in this
//! version. Each option of `codex exec` is described once below — its
//! spellings, how many words it takes and what it means for the session the
//! run makes — so the action is found whatever the order. Codex's own
//! meanings apply, not Claude's: `-c` is a configuration override and `-p` a
//! profile, each taking one value. Checked against `codex exec --help` of
//! Codex CLI 0.157.0 and its non-interactive documentation.
//!
//! Only [`Action::Create`] — `codex exec` (or its alias `e`) with `--json`,
//! no subcommand and at most one prompt word — is a launch. Resume, fork and
//! review, at the top level or under `exec`, an ephemeral run (it saves no
//! session), help and version are each their own action and none is a fresh
//! creation. A run without `--json` prints its identity only as text, which
//! is never read. An option not described here — an image list, a thread
//! source, a top-level option, `--` — leaves the command undecided: how many
//! words it takes, or what it does to the session, is not known.
//!
//! The command never names its child. The child is the one thread the run's
//! own first process result printed as `thread.started`
//! ([`thread_started`]), read as typed events and never by searching text
//! for an identifier.

use super::command::uuid;
use super::rows::LooseText;
use super::shell;
use serde::Deserialize;

/// How many words an option takes after its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Arity {
    /// None.
    Switch,
    /// One: the next word, or after `=` in a long option's own word.
    One,
}

/// What an option means for the session a `codex exec` run makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Meaning {
    /// `--json`: events on standard output, one JSON object per line.
    Json,
    /// No bearing on which session is created: configuration, features,
    /// model, provider, profile, sandbox, approvals, hooks, directories,
    /// worktree, rules, output files and colour.
    Neutral,
    /// Saves no session.
    Ephemeral,
    /// Prints help or the version and starts no session.
    Info,
}

struct Opt {
    names: &'static [&'static str],
    arity: Arity,
    meaning: Meaning,
}

const fn opt(names: &'static [&'static str], arity: Arity, meaning: Meaning) -> Opt {
    Opt {
        names,
        arity,
        meaning,
    }
}

use Arity::{One, Switch};
use Meaning::{Ephemeral, Info, Json, Neutral};

/// The `codex exec` options this map knows.
const OPTIONS: &[Opt] = &[
    opt(&["--json"], Switch, Json),
    // Codex's `-c` overrides one configuration value; it is not Claude's
    // continue. Codex's `-p` names a profile; it is not Claude's print.
    opt(&["-c", "--config"], One, Neutral),
    opt(&["--enable"], One, Neutral),
    opt(&["--disable"], One, Neutral),
    opt(&["--strict-config"], Switch, Neutral),
    opt(&["-m", "--model"], One, Neutral),
    opt(&["--oss"], Switch, Neutral),
    opt(&["--local-provider"], One, Neutral),
    opt(&["-p", "--profile"], One, Neutral),
    opt(&["-s", "--sandbox"], One, Neutral),
    opt(&["--approve-for-me"], Switch, Neutral),
    opt(
        &["--dangerously-bypass-approvals-and-sandbox"],
        Switch,
        Neutral,
    ),
    opt(&["--dangerously-bypass-hook-trust"], Switch, Neutral),
    opt(&["-C", "--cd"], One, Neutral),
    opt(&["--worktree"], Switch, Neutral),
    opt(&["--add-dir"], One, Neutral),
    opt(&["--skip-git-repo-check"], Switch, Neutral),
    opt(&["--ignore-user-config"], Switch, Neutral),
    opt(&["--ignore-rules"], Switch, Neutral),
    opt(&["--output-schema"], One, Neutral),
    opt(&["--color"], One, Neutral),
    opt(&["-o", "--output-last-message"], One, Neutral),
    opt(&["--ephemeral"], Switch, Ephemeral),
    opt(&["-h", "--help"], Switch, Info),
    opt(&["-V", "--version"], Switch, Info),
];

/// The action a `codex` command takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    /// `codex exec --json`: a fresh saved session, its events printed.
    Create,
    /// `codex exec` without `--json`: a fresh session whose identity is
    /// printed only as text, which is never read.
    Plain,
    /// Goes on with an existing session: no new child.
    Resume,
    /// A new session holding an existing session's history: not a fresh
    /// child.
    Fork,
    /// A code review run: not this proof's launch.
    Review,
    /// An ephemeral run: it saves no session.
    Unsaved,
    /// Help or the version: no session.
    NoSession,
}

/// The words `codex` and `codex exec` take as a command of their own.
fn subcommand(word: &str) -> Option<Action> {
    Some(match word {
        "resume" => Action::Resume,
        "fork" => Action::Fork,
        "review" => Action::Review,
        "help" => Action::NoSession,
        _ => return None,
    })
}

/// The action of a `codex` command's words after the program, or `None` when
/// it is not known.
pub(super) fn read(args: &[String]) -> Option<Action> {
    let (command, rest) = args.split_first()?;
    match command.as_str() {
        "exec" | "e" => {}
        "-h" | "--help" | "-V" | "--version" => return Some(Action::NoSession),
        other => return subcommand(other),
    }
    let (mut json, mut ephemeral, mut info, mut prompts) = (false, false, false, 0);
    let mut index = 0;
    while index < rest.len() {
        let word = &rest[index];
        index += 1;
        if word.starts_with('-') && word != "-" {
            // `--` ends the options; what follows is not read here.
            if word == "--" {
                return None;
            }
            let (name, inline) = match word.split_once('=') {
                Some((name, value)) if name.starts_with("--") => (name, Some(value)),
                _ => (word.as_str(), None),
            };
            let opt = OPTIONS.iter().find(|opt| opt.names.contains(&name))?;
            match (opt.arity, inline) {
                (Switch, None) => {}
                (Switch, Some(_)) => return None,
                (One, Some(value)) if !value.is_empty() => {}
                (One, Some(_)) => return None,
                (One, None) => {
                    let value = rest.get(index)?;
                    // A value that looks like an option is not one.
                    if value.is_empty() || value.starts_with('-') {
                        return None;
                    }
                    index += 1;
                }
            }
            match opt.meaning {
                Json => json = true,
                Ephemeral => ephemeral = true,
                Info => info = true,
                Neutral => {}
            }
            continue;
        }
        // The first operand may name a subcommand instead of a prompt.
        if prompts == 0
            && let Some(action) = subcommand(word)
        {
            return Some(if info { Action::NoSession } else { action });
        }
        prompts += 1;
        if prompts > 1 {
            return None;
        }
    }
    Some(if info {
        Action::NoSession
    } else if ephemeral {
        Action::Unsaved
    } else if json {
        Action::Create
    } else {
        Action::Plain
    })
}

/// The action of one literal `codex` command, or `None` for anything this
/// map does not know or that is not a plain `codex` command.
pub(super) fn action(command: &str) -> Option<Action> {
    let shell::Command {
        assignments,
        words,
        stdin: _,
        stdout,
        stderr: _,
    } = shell::command(command).ok()?;
    if assignments != 0 || stdout.is_some() {
        return None;
    }
    let (program, args) = words.split_first()?;
    if program != "codex" {
        return None;
    }
    read(args)
}

/// Whether one literal command is a fresh `codex exec --json` launch.
pub(super) fn launch(command: &str) -> bool {
    action(command) == Some(Action::Create)
}

/// The first printed JSON event, read whole: its type, and its thread when
/// it names one. Every other field is skipped unread.
#[derive(Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    thread_id: Option<LooseText>,
}

/// What reading a later event's top-level `type` found. Its other fields'
/// values, and anything nested, are skipped unread.
#[derive(Default)]
struct LaterType {
    kind: Option<String>,
    /// A second top-level `type` key was met.
    repeated: bool,
}

/// Reads one event object's top-level keys in order, keeping its `type`
/// as soon as it is decoded, so a cut or malformed field after it leaves
/// the type known.
struct TypeSeed<'a>(&'a mut LaterType);

impl<'de> serde::de::DeserializeSeed<'de> for TypeSeed<'_> {
    type Value = ();

    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_map(self)
    }
}

impl<'de> serde::de::Visitor<'de> for TypeSeed<'_> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an event object")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
        while let Some(key) = map.next_key::<std::borrow::Cow<'de, str>>()? {
            if key != "type" {
                map.next_value::<serde::de::IgnoredAny>()?;
                continue;
            }
            if self.0.kind.is_some() {
                self.0.repeated = true;
                return Err(serde::de::Error::custom("a second type"));
            }
            self.0.kind = Some(map.next_value::<String>()?);
        }
        Ok(())
    }
}

/// A later event line's top-level type, when it was decoded and is the only
/// one met: a cut or malformed payload after it does not matter; a type that
/// is missing, cut, not text or repeated, or a payload broken before it, is
/// no type.
fn later_type(line: &str) -> Option<String> {
    let mut found = LaterType::default();
    let mut deserializer = serde_json::Deserializer::from_str(line);
    let _ = serde::de::DeserializeSeed::deserialize(TypeSeed(&mut found), &mut deserializer);
    if found.repeated {
        return None;
    }
    found.kind
}

/// The thread a fresh `codex exec --json` run's printed output names: the
/// process result's `output`, as far as it had been printed. Only lines that
/// start as a JSON object are events; other lines (what the run wrote to
/// standard error) are not read, and nothing nested in an event is.
///
/// The first event must be one complete, valid `thread.started` naming a
/// full thread identity: it names the thread. Each later event is read only
/// for its top-level `type`, so a cut or malformed payload after a decoded
/// type (an output cut short, say) is ignored. A later event whose type is
/// `thread.started` — whatever its thread, complete or not — or whose type
/// is missing, cut, not text, repeated or after a broken payload names no
/// thread, and neither does output with no complete first start.
pub(super) fn thread_started(output: &str) -> Result<String, ()> {
    let mut thread = None;
    for line in output.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if !line.starts_with('{') {
            continue;
        }
        if thread.is_none() {
            let event: Event = serde_json::from_str(line).map_err(|_| ())?;
            let id = event.thread_id.and_then(|id| id.0).ok_or(())?;
            if event.kind != "thread.started" || !uuid(&id) {
                return Err(());
            }
            thread = Some(id);
            continue;
        }
        match later_type(line) {
            Some(kind) if kind != "thread.started" => {}
            _ => return Err(()),
        }
    }
    thread.ok_or(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const THREAD: &str = "01a00000-0000-7000-8000-0000000000c1";

    /// The real control's command shape, with an invented prompt.
    const CONTROL: &str = "codex exec --json --ignore-user-config --ignore-rules \
        --sandbox read-only --skip-git-repo-check -m gpt-6-sol \
        -c 'model_reasoning_effort=\"medium\"' 'Reply exactly OK. Do not use tools.'";

    #[test]
    fn reads_a_fresh_json_run_with_every_known_option_in_any_order() {
        assert_eq!(action(CONTROL), Some(Action::Create));
        for command in [
            "codex exec --json 'x'",
            "codex e --json 'x'",
            "codex exec 'x' --json",
            "codex exec --json",
            "codex exec --json -",
            "codex exec --json < /w/brief.md",
            "codex exec --json 'x' 2>&1",
            "codex exec --json 'x' 2> /w/e.log",
            "codex exec --json --model=m --sandbox=workspace-write 'x'",
            "codex exec --json -c a=1 -c b=2 --enable f --disable g --strict-config 'x'",
            "codex exec --json -p work --oss --local-provider ollama 'x'",
            "codex exec --json --approve-for-me --dangerously-bypass-approvals-and-sandbox 'x'",
            "codex exec --json --dangerously-bypass-hook-trust -C /w --worktree 'x'",
            "codex exec --json --add-dir /a --add-dir /b --output-schema /w/s.json 'x'",
            "codex exec --json --color never -o /w/last.txt 'x'",
        ] {
            assert_eq!(action(command), Some(Action::Create), "{command}");
            assert!(launch(command), "{command}");
        }
    }

    /// Codex's `-c` and `-p` take one value each: the word after them is
    /// never the prompt, and they never mean Claude's continue or print.
    #[test]
    fn codex_meanings_not_claudes() {
        assert_eq!(action("codex exec --json -c x=1 'p'"), Some(Action::Create));
        assert_eq!(
            action("codex exec --json -p prof 'p'"),
            Some(Action::Create)
        );
        // The word after `-c` is its value, never the prompt: here the
        // prompt is read from standard input.
        assert_eq!(action("codex exec --json -c 'p'"), Some(Action::Create));
        assert_eq!(action("codex exec --json -c"), None);
        assert_eq!(action("codex exec --json -p"), None);
        assert_eq!(action("codex exec --json -c -m m"), None);
    }

    #[test]
    fn keeps_every_other_action_apart() {
        for (command, expected) in [
            ("codex exec 'x'", Action::Plain),
            ("codex exec --json --ephemeral 'x'", Action::Unsaved),
            ("codex exec --ephemeral --json", Action::Unsaved),
            ("codex exec --json resume --last", Action::Resume),
            (
                "codex exec --json resume 01a00000-0000-7000-8000-0000000000c1 'x'",
                Action::Resume,
            ),
            ("codex exec resume --last --json 'x'", Action::Resume),
            (
                "codex exec --json fork 01a00000-0000-7000-8000-0000000000c1",
                Action::Fork,
            ),
            ("codex exec --json review", Action::Review),
            ("codex exec help", Action::NoSession),
            ("codex exec --json --help", Action::NoSession),
            ("codex exec --json -h 'x'", Action::NoSession),
            ("codex exec --json -V", Action::NoSession),
            ("codex exec --help resume", Action::NoSession),
            ("codex resume --last", Action::Resume),
            ("codex fork --last", Action::Fork),
            ("codex review", Action::Review),
            ("codex --help", Action::NoSession),
            ("codex --version", Action::NoSession),
            ("codex help", Action::NoSession),
        ] {
            assert_eq!(action(command), Some(expected), "{command}");
            assert!(!launch(command), "{command}");
        }
    }

    #[test]
    fn leaves_unknown_and_unsupported_forms_undecided() {
        for command in [
            // Not `codex`, a launcher path, a top-level option or an
            // interactive prompt.
            "claude -p --json 'x'",
            "/opt/homebrew/bin/codex exec --json 'x'",
            "./codex exec --json 'x'",
            "Codex exec --json 'x'",
            "codex",
            "codex 'x'",
            "codex -c a=1 exec --json 'x'",
            "codex --json exec 'x'",
            "codex login",
            "codex apply",
            "codex queue 'x'",
            // Options this map does not describe, or spelled wrongly.
            "codex exec --json -i /w/a.png 'x'",
            "codex exec --json --image /w/a.png 'x'",
            "codex exec --json --thread-source user 'x'",
            "codex exec --json --last 'x'",
            "codex exec --json -mgpt 'x'",
            "codex exec --json --json=1 'x'",
            "codex exec --json --model= 'x'",
            "codex exec --json --model '' 'x'",
            "codex exec --json -- 'x'",
            // Two prompts.
            "codex exec --json 'x' 'y'",
            "codex exec --json 'x' -",
            // Assignments, standard output redirected, shell structure.
            "CODEX_HOME=/w/h codex exec --json 'x'",
            "codex exec --json 'x' > /w/events.jsonl",
            "codex exec --json 'x' > /w/events.jsonl 2>&1",
            "codex exec --json 'x' | tee /w/o",
            "codex exec --json 'x'; true",
            "codex exec --json \"$(cat /w/brief)\"",
            "codex exec --json 'x' &",
        ] {
            assert_eq!(action(command), None, "{command}");
            assert!(!launch(command), "{command}");
        }
    }

    fn started(thread: &str) -> String {
        format!("{{\"type\":\"thread.started\",\"thread_id\":\"{thread}\"}}")
    }

    #[test]
    fn the_first_event_names_the_thread() {
        // The real control's first result: standard error, then two events.
        let control = format!(
            "Reading additional input from stdin...\n{}\n{{\"type\":\"turn.started\"}}\n",
            started(THREAD)
        );
        assert_eq!(thread_started(&control).as_deref(), Ok(THREAD));
        for output in [
            started(THREAD),
            format!("{}\r\n", started(THREAD)),
            format!(
                "{}\n{{\"type\":\"item.completed\",\"item\":{{\"id\":\"item_0\",\"text\":\"{}\"}}}}\n\
                 {{\"type\":\"turn.completed\",\"usage\":{{\"input_tokens\":1}}}}\n",
                started(THREAD),
                "01a00000-0000-7000-8000-0000000000ff"
            ),
            format!("warning: x\n{}\nlater stderr\n", started(THREAD)),
        ] {
            assert_eq!(thread_started(&output).as_deref(), Ok(THREAD), "{output}");
        }
    }

    /// A complete first start names the thread; a later event is read
    /// only for its top-level type, so its cut or malformed payload after a
    /// decoded type, and anything nested in it, does not matter.
    #[test]
    fn a_later_event_with_a_decoded_type_leaves_the_start() {
        for later in [
            // An output cut inside a later item's body.
            "{\"type\":\"item.completed\",\"item\":{\"id\":\"item_0\",\"text\":\"Long ans".to_owned(),
            "{\"type\":\"item.completed\",\"item\":{\"id\":".to_owned(),
            // A malformed payload after a decoded unrelated type.
            "{\"type\":\"turn.completed\",\"usage\":{\"input_tokens\":1,,}}".to_owned(),
            "{\"type\":\"turn.completed\"} trailing".to_owned(),
            // Start-like text nested in a body is never read as an event.
            format!(
                "{{\"type\":\"item.completed\",\"item\":{{\"text\":{}}}}}",
                serde_json::to_string(&started("01a00000-0000-7000-8000-0000000000c2")).unwrap()
            ),
            "{\"type\":\"item.completed\",\"item\":{\"type\":\"thread.started\",\"thread_id\":\"01a00000-0000-7000-8000-0000000000c2\"}}".to_owned(),
        ] {
            let output = format!("{}\n{later}", started(THREAD));
            assert_eq!(thread_started(&output).as_deref(), Ok(THREAD), "{output}");
        }
    }

    #[test]
    fn names_no_thread_unless_exactly_one_typed_first_event_does() {
        let other = "01a00000-0000-7000-8000-0000000000c2";
        for output in [
            // Nothing printed yet, or only standard error.
            String::new(),
            "Reading additional input from stdin...\n".into(),
            // A failure before any thread started.
            "Error: config.toml: unknown field\n".into(),
            "{\"type\":\"error\",\"message\":\"x\"}\n".into(),
            // A second start: another thread, the same one, or one whose
            // identity or body is cut.
            format!("{}\n{}\n", started(THREAD), started(other)),
            format!("{}\n{}\n", started(THREAD), started(THREAD)),
            format!(
                "{}\n{{\"type\":\"thread.started\",\"thread_id\":\"01a0",
                started(THREAD)
            ),
            format!("{}\n{{\"type\":\"thread.started\"", started(THREAD)),
            format!("{}\n{{\"type\":\"thread.started\",\"x\":", started(THREAD)),
            // A start that is not the first event.
            format!("{{\"type\":\"turn.started\"}}\n{}\n", started(THREAD)),
            // A later event whose type is cut, missing, not text, repeated,
            // or after a payload broken before it.
            format!("{}\n{{\"type\":\"turn.sta", started(THREAD)),
            format!("{}\n{{\"type\":", started(THREAD)),
            format!("{}\n{{", started(THREAD)),
            format!("{}\n{{\"kind\":\"x\"}}\n", started(THREAD)),
            format!("{}\n{{\"type\":7}}\n", started(THREAD)),
            format!(
                "{}\n{{\"type\":\"turn.started\",\"type\":\"x\"}}\n",
                started(THREAD)
            ),
            format!(
                "{}\n{{\"type\":\"turn.started\",\"type\":\"turn.started\"}}\n",
                started(THREAD)
            ),
            format!("{}\n{{\"item\":{{\"text\":\"cut", started(THREAD)),
            format!(
                "{}\n{{\"item\":,\"type\":\"turn.started\"}}\n",
                started(THREAD)
            ),
            // An incomplete or malformed first start.
            format!("{{\"type\":\n{}\n", started(THREAD)),
            format!(
                "{{\"type\":\"thread.started\",\"thread_id\":\"{}",
                &THREAD[..20]
            ),
            format!("{{\"type\":\"thread.started\",\"thread_id\":\"{THREAD}\"}} x\n"),
            format!("{{\"type\":\"thread.started\",\"type\":\"x\",\"thread_id\":\"{THREAD}\"}}\n"),
            // No identity, or not a full lowercase thread identity.
            "{\"type\":\"thread.started\"}\n".into(),
            "{\"type\":\"thread.started\",\"thread_id\":7}\n".into(),
            started(&THREAD.to_uppercase()),
            started("01a00000"),
            // An identity only mentioned, never typed.
            format!("session id: {THREAD}\n"),
            format!("{{\"type\":\"turn.started\",\"thread_id\":\"{THREAD}\"}}\n"),
            format!("x {}\n", started(THREAD)),
        ] {
            assert_eq!(thread_started(&output), Err(()), "{output}");
        }
    }
}
