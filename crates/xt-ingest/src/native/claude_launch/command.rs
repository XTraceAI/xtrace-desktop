//! The one launch form this version proves: a literal create-mode Claude
//! print command that names its new session with `--session-id` and submits
//! either one literal inline prompt or a prompt read from one redirected file.
//!
//! The command is read by the reviewed restricted shell reader: literal
//! words, possibly led by literal environment assignments of any valid name
//! (set aside: neither the program nor the prompt), at most one
//! standard-input and one standard-output redirection to an absolute path and
//! one standard-error redirection to an absolute path or standard output
//! (never opened) and nothing else. Its program is `claude` as the shell
//! finds it — taken on the recorded command's word, since no record says
//! which program that name ran — or one of the known installed Claude
//! launchers, present now as an executable file. Its options are read by the
//! shared option map ([`super::options`]), which both Claude launch readers
//! use: any known option in any order, and only a fresh print session
//! ([`Action::Create`]) is a launch. Its output format (text, JSON or
//! streaming JSON) and verbose logging change only what is printed, which
//! this proof never reads; the child is bound by its `--session-id`. A
//! command with a redirected standard input submits that file as its prompt
//! and must name no inline prompt; the file is never opened, so such a
//! launch has no prompt to compare. Resume, continue, fork, background, no
//! saved session, help, version, subcommands, structured input and every
//! unknown option refuse the command.

use super::options::{self, Action};
use super::shell;
use std::path::{Path, PathBuf};

/// What a launch command submits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Launch {
    /// The session it names, and so creates.
    pub child: String,
    /// The literal prompt word, compared in memory and never kept; `None`
    /// for a prompt read from a redirected file, which is never opened.
    pub prompt: Option<String>,
}

/// The installed Claude launchers a command may name, relative to the home
/// or absolute. A command naming any other path, even one ending in
/// `claude`, is not known to run Claude.
const LAUNCHERS: [&str; 7] = [
    "~/.local/bin/claude",
    "~/.claude/local/claude",
    "/opt/homebrew/bin/claude",
    "/usr/local/bin/claude",
    "~/.npm-global/bin/claude",
    "~/.bun/bin/claude",
    "~/.volta/bin/claude",
];

/// The known launcher paths under `home` that exist now as executable files.
pub(super) fn installed_programs(home: &Path) -> Vec<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    LAUNCHERS
        .iter()
        .map(|launcher| match launcher.strip_prefix("~/") {
            Some(relative) => home.join(relative),
            None => PathBuf::from(launcher),
        })
        .filter(|path| {
            std::fs::metadata(path)
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        })
        .collect()
}

/// A full lowercase UUID, as Claude spells a session.
pub(super) fn uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, &byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        })
}

/// The launch a literal command submits, or `None` for anything else.
pub(super) fn launch(command: &str, programs: &[PathBuf]) -> Option<Launch> {
    let shell::Command {
        assignments: _,
        words,
        stdin,
        stdout: _,
        stderr: _,
    } = shell::command(command).ok()?;
    let (program, args) = words.split_first()?;
    if program != "claude"
        && !programs
            .iter()
            .any(|verified| verified.as_os_str() == program.as_str())
    {
        return None;
    }
    let read = options::read(args)?;
    if read.action != Action::Create {
        return None;
    }
    // The prompt is the one literal word, or the redirected file, never both.
    let prompt = match (read.prompt, stdin) {
        (Some(prompt), None) if !prompt.is_empty() => Some(prompt),
        (None, Some(_)) => None,
        _ => return None,
    };
    Some(Launch {
        child: read.session_id?,
        prompt,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = "/opt/synthetic/bin/claude";
    const CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";
    const EMPTY_MCP: &str = r#"{"mcpServers":{}}"#;

    fn programs() -> Vec<PathBuf> {
        vec![PathBuf::from(CLAUDE)]
    }

    /// The launch shape a Codex coordinator writes, with every accepted
    /// option.
    fn full() -> String {
        format!(
            "{CLAUDE} -p --session-id {CHILD} --safe-mode --model claude-opus-5-5 --effort high \
             --permission-mode bypassPermissions --tools Read,Grep --allowedTools 'Bash(git:*)' \
             --disable-slash-commands --strict-mcp-config --mcp-config '{EMPTY_MCP}' \
             --output-format json 'Synthetic prompt, it'\\''s literal' > /w/result.json"
        )
    }

    #[test]
    fn reads_the_explicit_session_form_with_every_accepted_option() {
        assert_eq!(
            launch(&full(), &programs()),
            Some(Launch {
                child: CHILD.into(),
                prompt: Some("Synthetic prompt, it's literal".into()),
            })
        );
        assert_eq!(
            launch(
                &format!("{CLAUDE} --print 'p' --output-format json --session-id {CHILD}"),
                &programs()
            ),
            Some(Launch {
                child: CHILD.into(),
                prompt: Some("p".into())
            })
        );
    }

    #[test]
    fn refuses_every_other_launch() {
        let base = format!("{CLAUDE} -p --session-id {CHILD} --output-format json");
        for command in [
            // No session named, or named wrongly.
            format!("{CLAUDE} -p --output-format json 'x'"),
            format!(
                "{CLAUDE} -p --session-id {} --output-format json 'x'",
                CHILD.to_uppercase()
            ),
            format!("{CLAUDE} -p --session-id 8170544c --output-format json 'x'"),
            format!("{base} --session-id {CHILD} 'x'"),
            format!("{base} --session-id={CHILD} 'x'"),
            format!("{CLAUDE} -p --session-id= --output-format json 'x'"),
            // Resume, continue, fork, background, no saved session, help,
            // version, a subcommand, structured input.
            format!("{base} --resume {CHILD} 'x'"),
            format!("{CLAUDE} -p --resume {CHILD} --output-format json 'x'"),
            format!("{base} -r {CHILD} 'x'"),
            format!("{base} --continue 'x'"),
            format!("{base} -c 'x'"),
            format!("{base} --fork-session 'x'"),
            format!("{base} --bg 'x'"),
            format!("{base} --background 'x'"),
            format!("{base} --no-session-persistence 'x'"),
            format!("{base} --help"),
            format!("{base} --version"),
            format!("{base} -v"),
            format!("{base} doctor"),
            format!("{base} --input-format stream-json 'x'"),
            format!("{base} -- 'x'"),
            // An unknown output format, print missing or repeated, no or two
            // prompts.
            format!("{CLAUDE} -p --session-id {CHILD} --output-format yaml 'x'"),
            format!("{CLAUDE} -p --session-id {CHILD} --output-format 'x'"),
            // Values and prerequisites the CLI refuses before any session.
            format!("{CLAUDE} -p --session-id {CHILD} --output-format stream-json 'x'"),
            format!("{base} --max-budget-usd nope 'x'"),
            format!("{base} --max-budget-usd 0 'x'"),
            format!("{CLAUDE} --session-id {CHILD} --output-format json 'x'"),
            format!("{base} -p 'x'"),
            base.clone(),
            format!("{base} ''"),
            format!("{base} 'x' 'y'"),
            // An inline prompt and a redirected one; pipes, lists,
            // substitution, background.
            format!("{base} 'x' < /w/brief.md"),
            format!("{base} < brief.md"),
            format!("{base} < /dev/stdin"),
            format!("{base} 'x' | tee /w/o"),
            format!("{base} 'x'; true"),
            format!("{base} 'x' && true"),
            format!("{base} \"$(cat /w/brief)\""),
            format!("{base} 'x' &"),
            format!("{base} 'x' 2>> /w/e"),
            format!("{base} 'x' 2> /dev/null"),
            format!("{base} 'x' &> /w/e"),
            // Unknown or repeated options, values that look like options.
            format!("{base} --agent a 'x'"),
            format!("{base} --debug 'x'"),
            format!("{base} --verbose --verbose 'x'"),
            format!("{base} --name a -n b 'x'"),
            format!("{base} --model m --model n 'x'"),
            format!("{base} --model -x 'x'"),
            format!("{base} --safe-mode --safe-mode 'x'"),
            // A prompt that a list option would take as its second value.
            format!("{base} --mcp-config /w/mcp.json 'x'"),
            format!("{base} --mcp-config '{{\"mcpServers\":{{\"s\":{{}}}}}}' 'x'"),
            format!("{base} --mcp-config '{{}}' 'x'"),
            // An MCP configuration other than the empty-server one.
            format!("{base} --mcp-config '{{}}' --safe-mode 'x'"),
            format!("{base} --mcp-config /w/mcp.json --safe-mode 'x'"),
            format!("{base} --mcp-config '{{\"mcpServers\":{{\"s\":{{}}}}}}' --safe-mode 'x'"),
            format!("{base} --tools Read 'x'"),
            format!("{base} --allowedTools Read 'x'"),
            format!("{base} --mcp-config '{EMPTY_MCP}' 'x'"),
            // Programs that are not a known launcher or the bare name.
            format!("/tmp/claude -p --session-id {CHILD} --output-format json 'x'"),
            format!("./claude -p --session-id {CHILD} --output-format json 'x'"),
            format!("Claude -p --session-id {CHILD} --output-format json 'x'"),
            // A name or browser option of the wrong arity.
            format!("{base} --name 'x'"),
            format!("{base} --name --no-chrome 'x'"),
            format!("{base} --no-chrome x 'y'"),
            format!("{base} --no-chrome --no-chrome 'x'"),
            // Resume, continue or fork beside a redirected prompt.
            format!("{CLAUDE} -p --resume {CHILD} --output-format json < /w/brief.md"),
            format!("{base} --continue < /w/brief.md"),
            format!("{base} --fork-session < /w/brief.md"),
            // Assignments that are not literal, or not leading, or not ones.
            format!("'FOO'=bar {base} 'x'"),
            format!("FOO'='bar {base} 'x'"),
            format!("\"FOO\"=bar {base} 'x'"),
            format!("1FOO=bar {base} 'x'"),
            format!("FOO-X=bar {base} 'x'"),
            format!("FOO+=bar {base} 'x'"),
            format!("FOO[1]=bar {base} 'x'"),
            format!("FOO=(a b) {base} 'x'"),
            format!("FOO=$HOME {base} 'x'"),
            format!("FOO=`id` {base} 'x'"),
            format!("FOO=~/x {base} 'x'"),
            format!("FOO==x {base} 'x'"),
            format!("env FOO=bar {base} 'x'"),
            format!("export FOO=bar; {base} 'x'"),
            format!("FOO=bar; {base} 'x'"),
            format!("FOO=bar && {base} 'x'"),
            format!("FOO=bar > /w/o {base} 'x'"),
            "FOO=bar".to_string(),
        ] {
            assert_eq!(launch(&command, &programs()), None, "{command}");
        }
        // A list option followed by another option keeps the prompt its own.
        assert!(launch(&format!("{base} --tools Read --safe-mode 'x'"), &programs()).is_some());
        assert!(launch(&format!("{base} 'x' --tools Read"), &programs()).is_some());
    }

    /// Literal assignments of any valid name lead the same launch; words
    /// after the launcher that look like assignments are its arguments.
    #[test]
    fn literal_assignments_may_lead_the_launch() {
        let base = format!("{CLAUDE} -p --session-id {CHILD} --output-format json");
        let expected = Some(Launch {
            child: CHILD.into(),
            prompt: Some("x".into()),
        });
        for prefix in [
            "FOO=bar",
            "_=1",
            "a_B9=/w/t",
            "X= Y=''",
            "Q='a b' R='it'\\''s' S=/w/x:/w/y T=a=b",
        ] {
            let command = format!("{prefix} {base} 'x' > /w/o.json");
            assert_eq!(launch(&command, &programs()), expected, "{command}");
        }
        // Words after the launcher that look like assignments are ordinary.
        assert_eq!(
            launch(&format!("{base} 'A=b'"), &programs()),
            Some(Launch {
                child: CHILD.into(),
                prompt: Some("A=b".into())
            })
        );
        assert_eq!(
            launch(&format!("{base} A=b"), &programs()).map(|launch| launch.prompt),
            Some(Some("A=b".into()))
        );
        assert_eq!(launch(&format!("{base} 'x' A=b"), &programs()), None);
    }

    /// The coordinator's own job form: the bare program, a session title, no
    /// browser, a prompt read from a file that is never opened, and the JSON
    /// result redirected. No prompt is read from it.
    #[test]
    fn reads_the_bare_named_launch_with_a_redirected_prompt() {
        let stdin = Some(Launch {
            child: CHILD.into(),
            prompt: None,
        });
        for command in [
            format!(
                "claude -p --session-id {CHILD} --name 'Fix active session names' --no-chrome \
                 --model claude-opus-5-5 --effort high --permission-mode bypassPermissions \
                 --output-format json < /w/brief.md > /w/job.json"
            ),
            format!(
                "claude --name \"Hide unlinked sub-sessions\" --no-chrome -p --session-id {CHILD} \
                 --output-format json < /w/brief.md"
            ),
            format!("{CLAUDE} -p --output-format json --session-id {CHILD} < /w/brief.md"),
        ] {
            assert_eq!(launch(&command, &programs()), stdin, "{command}");
        }
        // The bare program with an inline prompt keeps that prompt.
        assert_eq!(
            launch(
                &format!("claude -p --session-id {CHILD} --output-format json --no-chrome 'x'"),
                &[]
            ),
            Some(Launch {
                child: CHILD.into(),
                prompt: Some("x".into()),
            })
        );
        // A redirected prompt with no session named, or a session named
        // with resume, is no create launch.
        for command in [
            "claude -p --output-format json --name n < /w/brief.md".to_owned(),
            format!("claude -p -r {CHILD} --output-format json < /w/brief.md"),
        ] {
            assert_eq!(launch(&command, &programs()), None, "{command}");
        }
    }

    /// The coordinator job shape version 6 refused: the empty-server MCP
    /// configuration, every option ahead of `--print`, verbose logging,
    /// streaming JSON output and standard error redirected to its own file.
    /// Only the session it names and its redirected prompt matter. The same
    /// launch with the MCP configuration `{}`, which Claude refuses before
    /// saving any session, is no launch.
    #[test]
    fn reads_the_streaming_verbose_launch_with_its_errors_redirected() {
        let command = format!(
            "claude --safe-mode --no-chrome --model claude-opus-5-5 --effort high \
             --name 'Synthetic task title' --session-id {CHILD} --permission-mode acceptEdits \
             --tools 'Bash,Read,Edit,Write,Glob,Grep' --allowedTools 'Bash,Read,Edit,Write,Glob,Grep' \
             --strict-mcp-config --mcp-config '{EMPTY_MCP}' --print --verbose --output-format stream-json \
             < /w/prompt.txt > /w/job.jsonl 2> /w/job.stderr"
        );
        assert_eq!(
            launch(&command, &[]),
            Some(Launch {
                child: CHILD.into(),
                prompt: None,
            })
        );
        let rejected = command.replace(EMPTY_MCP, "{}");
        assert_ne!(rejected, command);
        assert_eq!(launch(&rejected, &[]), None);
    }

    /// The output format, verbose logging, standard-error routing and the
    /// `=` spelling of a value change nothing about which session a launch
    /// names or its prompt.
    #[test]
    fn output_format_logging_and_routing_are_not_creation_evidence() {
        let expected = Some(Launch {
            child: CHILD.into(),
            prompt: Some("x".into()),
        });
        for tail in [
            "",
            "--output-format text",
            "--output-format json",
            "--output-format stream-json --verbose",
            "--verbose",
            "--verbose --output-format=json",
            "--output-format=stream-json --verbose",
        ] {
            for routing in [
                "",
                " > /w/o",
                " 2> /w/e",
                " 2>&1",
                " > /w/o 2>&1",
                " > /w/o 2> /w/e",
            ] {
                for session in [
                    format!("--session-id {CHILD}"),
                    format!("--session-id={CHILD}"),
                ] {
                    let command =
                        format!("{CLAUDE} -p {session} {tail} 'x'{routing}").replace("  ", " ");
                    assert_eq!(launch(&command, &programs()), expected, "{command}");
                }
            }
        }
    }

    #[test]
    fn only_known_launchers_present_as_executables_are_programs() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::TempDir::new().unwrap();
        let home = temp.path();
        assert!(!installed_programs(home).iter().any(|p| p.starts_with(home)));
        let bin = home.join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let launcher = bin.join("claude");
        std::fs::write(&launcher, "#!/bin/sh\n").unwrap();
        assert!(
            !installed_programs(home).contains(&launcher),
            "not executable"
        );
        std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(installed_programs(home).contains(&launcher));
        let other = home.join("bin/claude");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, "").unwrap();
        std::fs::set_permissions(&other, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!installed_programs(home).contains(&other));
    }
}
