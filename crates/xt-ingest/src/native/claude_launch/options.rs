//! The one maintained description of the Claude CLI options the two launch
//! readers understand: the Codex explicit-session reader ([`super::command`])
//! and the Claude `Bash` script reader (`bash::script`). Each option is
//! described once — its spellings, how many words it takes and what it means
//! for the session a print launch makes — and [`read`] turns a launch's
//! words, in any order, into the action it takes, the session it names, what
//! it prints and its prompt. It holds no command templates.
//!
//! Checked against `claude --help` of Claude Code 2.1.284; `--max-turns` is
//! accepted there but not listed. An option not described here is unknown:
//! how many words it takes is not known, so neither is where the prompt is,
//! and the launch is undecided. Options that only choose the model, effort,
//! permissions, tools, title, prompt additions or configuration are neutral:
//! their values never decide which session is created — but a value the CLI
//! refuses before starting any session is never read as a launch: an MCP
//! configuration is read only as the literal empty-server one
//! ([`Meaning::McpConfig`]), a budget only as a positive amount
//! ([`Meaning::Budget`]), and streaming JSON output only with `--verbose`
//! ([`Action::Invalid`]). Output format and verbose logging otherwise change
//! only what is printed. Resume,
//! continue, fork, background, no saved session, help, version and a
//! subcommand each decide the action; only [`Action::Create`] is a fresh
//! print session, and neither reader proves any other action.

/// How many words an option takes after its name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Arity {
    /// None.
    Switch,
    /// One: the next word, or after `=` in a long option's own word.
    One,
    /// One or more: the CLI takes every following word that does not start
    /// with `-`. Read with exactly one value; a plain word after it would be
    /// another value, never the prompt, so it refuses the launch.
    Many,
    /// One if the next word does not start with `-`, as the CLI reads it.
    Optional,
}

/// What an option means for the session a launch makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Meaning {
    /// `-p`: one turn, printed, then exit.
    Print,
    /// Names the session with a full identifier.
    SessionId,
    /// What the turn prints: [`Format`].
    OutputFormat,
    /// `text` is one prompt; `stream-json` is structured input.
    InputFormat,
    /// A positive turn limit.
    MaxTurns,
    /// A spending limit. Claude refuses anything but a positive number
    /// before starting any session; only a plain positive decimal is read.
    Budget,
    /// More detail in what is printed.
    Verbose,
    /// No bearing on which session is created.
    Neutral,
    /// MCP servers to load. Only the literal empty-server configuration is
    /// read: Claude refuses an invalid one (`{}`, say) before saving any
    /// session, a launch that fails so is no creation, and a file or another
    /// configuration is never opened or checked here.
    McpConfig,
    /// Goes on with a named existing session.
    Resume,
    /// Goes on with the directory's most recent session.
    Continue,
    /// Copies the resumed session's history under a new identifier.
    Fork,
    /// Returns at once; with resume, continues that session or, when it is
    /// already running, starts a copy.
    Background,
    /// Saves no session.
    NoPersistence,
    /// Prints help or the version and starts no session.
    Info,
}

/// One option: every spelling, its arity and its meaning.
#[derive(Debug)]
pub(super) struct Opt {
    pub names: &'static [&'static str],
    pub arity: Arity,
    pub meaning: Meaning,
}

const fn opt(names: &'static [&'static str], arity: Arity, meaning: Meaning) -> Opt {
    Opt {
        names,
        arity,
        meaning,
    }
}

use Arity::{Many, One, Optional, Switch};
use Meaning::*;

/// The options both readers know.
pub(super) const OPTIONS: &[Opt] = &[
    opt(&["-p", "--print"], Switch, Print),
    opt(&["--session-id"], One, SessionId),
    opt(&["--output-format"], One, OutputFormat),
    opt(&["--input-format"], One, InputFormat),
    opt(&["--max-turns"], One, MaxTurns),
    opt(&["--verbose"], Switch, Verbose),
    // Neutral: model, effort, permissions, title, prompt additions,
    // configuration, tools.
    opt(&["--model"], One, Neutral),
    opt(&["--fallback-model"], One, Neutral),
    opt(&["--effort"], One, Neutral),
    opt(&["--permission-mode"], One, Neutral),
    opt(&["-n", "--name"], One, Neutral),
    opt(&["--system-prompt"], One, Neutral),
    opt(&["--append-system-prompt"], One, Neutral),
    opt(&["--settings"], One, Neutral),
    opt(&["--setting-sources"], One, Neutral),
    opt(&["--max-budget-usd"], One, Budget),
    opt(&["--tools"], Many, Neutral),
    opt(&["--allowedTools", "--allowed-tools"], Many, Neutral),
    opt(&["--disallowedTools", "--disallowed-tools"], Many, Neutral),
    opt(&["--add-dir"], Many, Neutral),
    opt(&["--mcp-config"], Many, McpConfig),
    opt(&["--dangerously-skip-permissions"], Switch, Neutral),
    opt(&["--allow-dangerously-skip-permissions"], Switch, Neutral),
    opt(&["--safe-mode"], Switch, Neutral),
    opt(&["--disable-slash-commands"], Switch, Neutral),
    opt(&["--strict-mcp-config"], Switch, Neutral),
    opt(&["--no-chrome"], Switch, Neutral),
    // The actions that are not a fresh session.
    opt(&["-r", "--resume"], Optional, Resume),
    opt(&["--from-pr"], Optional, Resume),
    opt(&["--teleport"], Optional, Resume),
    opt(&["-c", "--continue"], Switch, Continue),
    opt(&["--fork-session"], Switch, Fork),
    opt(&["--bg", "--background"], Switch, Background),
    opt(&["--no-session-persistence"], Switch, NoPersistence),
    opt(&["-h", "--help"], Switch, Info),
    opt(&["-v", "--version"], Switch, Info),
];

/// Commands the CLI runs instead of a session when its first operand names
/// one, so a prompt word spelled as one starts no session.
const SUBCOMMANDS: &[&str] = &[
    "agents",
    "attach",
    "auth",
    "auto-mode",
    "doctor",
    "gateway",
    "import",
    "install",
    "kill",
    "logs",
    "mcp",
    "plugin",
    "plugins",
    "project",
    "respawn",
    "rm",
    "setup-token",
    "stop",
    "ultrareview",
    "update",
    "upgrade",
];

/// What a print turn writes to standard output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Format {
    /// The plain answer (the CLI's default).
    Text,
    /// One JSON result.
    Json,
    /// One JSON event per line.
    StreamJson,
}

/// The action a launch takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Action {
    /// One print turn in a fresh saved session.
    Create,
    /// Goes on with an existing session: no new child.
    Resume,
    /// A new identifier holding an existing session's history: not a fresh
    /// child.
    Fork,
    /// A background launch: alone it returns at once, with resume it may
    /// continue that session or start a copy. Conditional either way.
    Background,
    /// A print turn that saves no session.
    Unsaved,
    /// Help, version or a subcommand: no session.
    NoSession,
    /// No print turn of one prompt: interactive, or structured input.
    NotPrint,
    /// A print turn the CLI refuses before starting any session: streaming
    /// JSON output without `--verbose`.
    Invalid,
}

/// What a launch's words say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Read {
    pub action: Action,
    /// The identifier `--session-id` gave.
    pub session_id: Option<String>,
    pub format: Format,
    pub verbose: bool,
    /// The one prompt word, if any.
    pub prompt: Option<String>,
}

/// Whether an MCP configuration is the literal empty-server one:
/// `{"mcpServers":{}}` with any JSON whitespace (space, tab, carriage return,
/// line feed) between and around its tokens. See [`mcp_shape`].
fn empty_mcp(value: &str) -> bool {
    mcp_shape(value.as_bytes()) == (value.len(), true)
}

/// How far `value` follows the empty-server shape, read in place with
/// constant space: the bytes read, and whether the whole shape was matched.
/// It stops at the first byte that does not fit, so nothing else is parsed,
/// decoded or allocated. An escaped key, a duplicate key, another member, a
/// non-empty or non-object value and trailing data do not fit.
fn mcp_shape(value: &[u8]) -> (usize, bool) {
    const TOKENS: [&[u8]; 6] = [b"{", b"\"mcpServers\"", b":", b"{", b"}", b"}"];
    let space = |at: usize| {
        at + value[at..]
            .iter()
            .take_while(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n'))
            .count()
    };
    let mut at = 0;
    for token in TOKENS {
        at = space(at);
        if !value[at..].starts_with(token) {
            return (at, false);
        }
        at += token.len();
    }
    (space(at), true)
}

/// Whether a budget is a plain positive decimal (`5`, `0.5`, `2.50`). A
/// sign, exponent, other notation or zero is not read.
fn positive_amount(value: &str) -> bool {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, "0"));
    !whole.is_empty()
        && !fraction.is_empty()
        && whole
            .bytes()
            .chain(fraction.bytes())
            .all(|byte| byte.is_ascii_digit())
        && value
            .parse::<f64>()
            .is_ok_and(|amount| amount.is_finite() && amount > 0.0)
}

/// The option a word spells, by its place in [`OPTIONS`], and the value it
/// carries after `=`.
fn lookup(word: &str) -> Option<(usize, Option<&str>)> {
    let (name, inline) = match word.split_once('=') {
        Some((name, value)) if name.starts_with("--") => (name, Some(value)),
        Some(_) => return None,
        None => (word, None),
    };
    let index = OPTIONS.iter().position(|opt| opt.names.contains(&name))?;
    Some((index, inline))
}

/// What a launch's words (everything after the program) say, or `None` when
/// they are undecided: an unknown or repeated option, a malformed value, a
/// value an option needs missing or starting with `-`, `--`, two prompt
/// words, or a plain word after a many-valued option's value.
pub(super) fn read(args: &[String]) -> Option<Read> {
    let mut seen: Vec<usize> = Vec::new();
    let mut prompt: Option<&String> = None;
    let mut session_id = None;
    let mut format = Format::Text;
    let mut structured = false;
    let mut meanings: Vec<Meaning> = Vec::new();
    let mut after_many = false;
    let mut args = args.iter().peekable();
    while let Some(arg) = args.next() {
        let many_before = std::mem::take(&mut after_many);
        if !arg.starts_with('-') {
            if many_before || prompt.replace(arg).is_some() {
                return None;
            }
            continue;
        }
        let (index, inline) = lookup(arg)?;
        if seen.contains(&index) {
            return None;
        }
        seen.push(index);
        let opt = &OPTIONS[index];
        let value = match (opt.arity, inline) {
            (Switch, None) => None,
            (Switch, Some(_)) => return None,
            (One | Many | Optional, Some(value)) => Some(value),
            (One | Many, None) => Some(args.next()?.as_str()),
            (Optional, None) => args
                .next_if(|next| !next.starts_with('-'))
                .map(String::as_str),
        };
        if opt.arity == Many {
            after_many = true;
        }
        match (opt.meaning, value) {
            (SessionId, Some(value)) if super::command::uuid(value) => {
                session_id = Some(value.to_owned());
            }
            (OutputFormat, Some(value)) => {
                format = match value {
                    "text" => Format::Text,
                    "json" => Format::Json,
                    "stream-json" => Format::StreamJson,
                    _ => return None,
                };
            }
            (InputFormat, Some("text")) => {}
            (InputFormat, Some("stream-json")) => structured = true,
            (MaxTurns, Some(value))
                if !value.is_empty()
                    && value.bytes().all(|byte| byte.is_ascii_digit())
                    && value.parse::<u32>().is_ok_and(|n| n > 0) => {}
            (Budget, Some(value)) if positive_amount(value) => {}
            (Neutral, Some(value)) if opt.arity == One => {
                if value.is_empty() || value.starts_with('-') {
                    return None;
                }
            }
            // A many-valued option's value may be empty (`--tools ""`).
            (Neutral, Some(value)) if !value.starts_with('-') => {}
            (McpConfig, Some(value)) if empty_mcp(value) => {}
            (
                Neutral | Print | Verbose | Continue | Fork | Background | NoPersistence | Info,
                None,
            ) => {}
            (Resume, _) => {}
            _ => return None,
        }
        meanings.push(opt.meaning);
    }
    let has = |meaning| meanings.contains(&meaning);
    let action = if has(Info) || prompt.is_some_and(|word| SUBCOMMANDS.contains(&word.as_str())) {
        Action::NoSession
    } else if has(Print) && format == Format::StreamJson && !has(Verbose) {
        Action::Invalid
    } else if has(Fork) {
        Action::Fork
    } else if has(Background) {
        Action::Background
    } else if has(Resume) || has(Continue) {
        Action::Resume
    } else if has(NoPersistence) {
        Action::Unsaved
    } else if !has(Print) || structured {
        Action::NotPrint
    } else {
        Action::Create
    };
    Some(Read {
        action,
        session_id,
        format,
        verbose: has(Verbose),
        prompt: prompt.cloned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";

    fn words(line: &str) -> Vec<String> {
        line.split(' ').map(str::to_owned).collect()
    }

    fn action(line: &str) -> Option<Action> {
        read(&words(line)).map(|read| read.action)
    }

    #[test]
    fn every_spelling_is_described_once() {
        let mut names: Vec<&str> = OPTIONS
            .iter()
            .flat_map(|opt| opt.names.iter().copied())
            .collect();
        let all = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), all, "a spelling described twice");
        for name in names {
            assert!(
                name.starts_with("--") || (name.len() == 2 && name.starts_with('-')),
                "{name}"
            );
            assert!(!name.contains('='), "{name}");
        }
    }

    #[test]
    fn neutral_options_in_any_order_and_spelling_leave_a_create() {
        for line in [
            format!("-p --session-id {CHILD} x"),
            format!("x --session-id={CHILD} --print"),
            "--model opus -p --effort=high -n title x".to_owned(),
            r#"-p x --name=title --tools Read,Grep --allowed-tools Bash --mcp-config {"mcpServers":{}}"#
                .to_owned(),
            "-p --tools= --disallowedTools Write --add-dir /w --safe-mode x".to_owned(),
            "-p --max-turns 3 --input-format text --fallback-model sonnet x".to_owned(),
            "-p --settings /w/s.json --setting-sources user --max-budget-usd 1 x".to_owned(),
            "-p --max-budget-usd=0.5 x".to_owned(),
            "-p --max-budget-usd 2.50 x".to_owned(),
            "-p --system-prompt s --append-system-prompt t --no-chrome x".to_owned(),
            "-p --allow-dangerously-skip-permissions --dangerously-skip-permissions x".to_owned(),
            "-p --verbose --output-format stream-json".to_owned(),
        ] {
            assert_eq!(action(&line), Some(Action::Create), "{line}");
        }
    }

    #[test]
    fn output_format_and_verbose_are_read_not_required() {
        let format = |line: &str| read(&words(line)).map(|read| (read.format, read.verbose));
        assert_eq!(format("-p x"), Some((Format::Text, false)));
        assert_eq!(
            format("-p --output-format text x"),
            Some((Format::Text, false))
        );
        assert_eq!(
            format("-p --output-format json x"),
            Some((Format::Json, false))
        );
        assert_eq!(
            format("-p --verbose --output-format=stream-json x"),
            Some((Format::StreamJson, true))
        );
        assert_eq!(format("-p --output-format yaml x"), None);
        assert_eq!(format("-p --output-format"), None);
    }

    #[test]
    fn actions_other_than_create_are_told_apart() {
        for (line, expected) in [
            (format!("-p --resume {CHILD} x"), Action::Resume),
            (format!("-p --resume={CHILD} x"), Action::Resume),
            ("-p -r -c".to_owned(), Action::Resume),
            ("-p --continue x".to_owned(), Action::Resume),
            ("-p --from-pr 12 --model m x".to_owned(), Action::Resume),
            // `-r` takes the next plain word as its value, as the CLI does.
            ("-p -r x".to_owned(), Action::Resume),
            (
                format!("-p -r {CHILD} --fork-session --session-id {CHILD} x"),
                Action::Fork,
            ),
            ("-p -c --fork-session x".to_owned(), Action::Fork),
            (format!("-p --bg --resume {CHILD} x"), Action::Background),
            ("-p --background x".to_owned(), Action::Background),
            ("-p --no-session-persistence x".to_owned(), Action::Unsaved),
            ("-p --help".to_owned(), Action::NoSession),
            ("-v".to_owned(), Action::NoSession),
            ("-p --resume x --help".to_owned(), Action::NoSession),
            ("-p doctor".to_owned(), Action::NoSession),
            ("-p --model m mcp".to_owned(), Action::NoSession),
            ("x".to_owned(), Action::NotPrint),
            (format!("--session-id {CHILD} x"), Action::NotPrint),
            ("-p --input-format stream-json".to_owned(), Action::NotPrint),
            // The CLI refuses print streaming output without `--verbose`.
            (
                "-p --output-format stream-json x".to_owned(),
                Action::Invalid,
            ),
            (
                "x --output-format=stream-json --print".to_owned(),
                Action::Invalid,
            ),
            (
                format!("-p --session-id {CHILD} --output-format stream-json x"),
                Action::Invalid,
            ),
        ] {
            assert_eq!(action(&line), Some(expected), "{line}");
        }
        // The prompt word stays the prompt beside an action.
        assert_eq!(
            read(&words("-p --continue x")).unwrap().prompt.as_deref(),
            Some("x")
        );
    }

    /// Only the literal empty-server MCP configuration is read, with any
    /// JSON whitespace around its tokens; `{}` (which Claude refuses before
    /// saving any session), escaped or duplicate keys, other members,
    /// non-empty values, trailing data and files are not.
    #[test]
    fn only_the_empty_server_mcp_configuration_is_read() {
        let with = |config: &str| {
            let args: Vec<String> = ["-p", "--mcp-config", config, "--safe-mode", "x"]
                .map(str::to_owned)
                .to_vec();
            read(&args).map(|read| read.action)
        };
        // Every JSON whitespace character, alone or mixed, at every gap.
        let mut accepted = vec![r#"{"mcpServers":{}}"#.to_owned()];
        for space in [" ", "\t", "\r", "\n", " \t\r\n", "\r\n  "] {
            accepted.push(format!(
                "{space}{{{space}\"mcpServers\"{space}:{space}{{{space}}}{space}}}{space}"
            ));
            for gap in 0..7 {
                let mut config = String::new();
                for (index, token) in ["{", "\"mcpServers\"", ":", "{", "}", "}", ""]
                    .iter()
                    .enumerate()
                {
                    if index == gap {
                        config.push_str(space);
                    }
                    config.push_str(token);
                }
                accepted.push(config);
            }
        }
        for config in &accepted {
            assert!(empty_mcp(config), "{config:?}");
            assert_eq!(with(config), Some(Action::Create), "{config:?}");
            assert_eq!(
                read(&[format!("--mcp-config={config}"), "-p".into(), "x".into()])
                    .map(|read| read.action),
                Some(Action::Create),
                "{config:?}"
            );
        }
        for config in [
            "{}",
            "",
            " ",
            "/w/mcp.json",
            "mcp.json",
            r#"{"mcpServers":{"s":{"command":"x"}}}"#,
            r#"{"mcpServers":[]}"#,
            r#"{"mcpServers":[1,2,3]}"#,
            r#"{"mcpServers":null}"#,
            r#"{"mcpServers":{},"other":1}"#,
            r#"{"other":1,"mcpServers":{}}"#,
            r#"{"mcpServers":{},"mcpServers":{}}"#,
            r#"{"mcp\u0053ervers":{}}"#,
            r#"{"mcpServers\u0020":{}}"#,
            r#"{"servers":{}}"#,
            r#"{'mcpServers':{}}"#,
            r#"[{"mcpServers":{}}]"#,
            r#"{"mcpServers":{}"#,
            r#"{"mcpServers":{}}}"#,
            r#"{"mcpServers":{}} x"#,
            r#"{"mcpServers":{}},"#,
            "{\u{a0}\"mcpServers\":{}}",
            "{\u{b}\"mcpServers\":{}}",
        ] {
            assert!(!empty_mcp(config), "{config:?}");
            assert_eq!(with(config), None, "{config:?}");
        }
    }

    /// A large invalid configuration is refused where it first leaves the
    /// shape, a few bytes in: what follows is never read, decoded or kept.
    #[test]
    fn a_large_invalid_mcp_configuration_is_refused_at_its_first_wrong_byte() {
        let dense = format!(r#"{{"mcpServers":[{}0]}}"#, "0,".repeat(4 << 20));
        assert_eq!(mcp_shape(dense.as_bytes()), (14, false));
        assert!(!empty_mcp(&dense));
        let nested = format!(r#"{{"mcpServers":{{"s":"{}"}}}}"#, "x".repeat(4 << 20));
        assert_eq!(mcp_shape(nested.as_bytes()), (15, false));
        let trailing = format!(r#"{{"mcpServers":{{}}}}{}"#, " ".repeat(4 << 20) + "x");
        assert_eq!(mcp_shape(trailing.as_bytes()), (trailing.len() - 1, true));
        assert!(!empty_mcp(&trailing));
    }

    #[test]
    fn unknown_repeated_or_malformed_options_are_undecided() {
        for line in [
            // Unknown: arity unknown, so the prompt cannot be found.
            "-p --agent a x",
            "-p --frobnicate x",
            "-p -d x",
            "-p -px",
            "-p -- x",
            "-p --worktree x",
            // Repeated, also through an alias.
            "-p -p x",
            "-p --name a -n b x",
            "-p --allowedTools a --allowed-tools b",
            "-p --verbose --verbose x",
            // Malformed values.
            "-p --session-id 8170544c x",
            "-p --session-id 0C000000-0000-4000-8000-0000000000C1 x",
            "-p --max-turns 0 x",
            "-p --max-turns -1 x",
            "-p --max-turns 1x x",
            "-p --max-budget-usd nope x",
            "-p --max-budget-usd 0 x",
            "-p --max-budget-usd 0.00 x",
            "-p --max-budget-usd -1 x",
            "-p --max-budget-usd +1 x",
            "-p --max-budget-usd 1e2 x",
            "-p --max-budget-usd 1. x",
            "-p --max-budget-usd .5 x",
            "-p --max-budget-usd 0x10 x",
            "-p --max-budget-usd= x",
            "-p --input-format yaml x",
            "-p --model -x x",
            "-p --model= x",
            "-p --name",
            "-p --tools -x x",
            // A switch given a value; a short option given one by `=`.
            "-p --verbose=1 x",
            "-p=1 x",
            "-p -n=t x",
            // Two prompts; a plain word a many-valued option would take.
            "-p x y",
            "-p --tools Read x",
            r#"-p --mcp-config {"mcpServers":{}} x"#,
            "-p --tools=Read x",
        ] {
            assert_eq!(read(&words(line)), None, "{line}");
        }
        // A many-valued option before another option, or last, is fine.
        assert_eq!(
            action("-p --tools Read --safe-mode x"),
            Some(Action::Create)
        );
        assert_eq!(action("-p x --tools Read"), Some(Action::Create));
    }
}
