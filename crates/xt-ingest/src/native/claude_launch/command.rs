//! The one launch form this version proves: a literal create-mode Claude
//! print command that names its new session with `--session-id` and submits
//! one literal inline prompt.
//!
//! The command is read by the reviewed restricted shell reader: literal
//! words, possibly led by literal environment assignments of any valid name
//! (set aside: neither the program nor the prompt), at most one
//! standard-output redirection to an absolute path (never opened) and
//! nothing else. Its program must be one of the known installed Claude
//! launchers, present now as an executable file. Its options are a closed
//! set with fixed arity; `--tools`, `--allowedTools` and `--mcp-config` may
//! take several values in the Claude CLI, so the word after their one value
//! must be another option or the end, and the prompt is never read as one of
//! their values. Resume, continue, fork, help, version, structured input and
//! every other option refuse the command.

use super::shell;
use std::path::{Path, PathBuf};

/// What a launch command submits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Launch {
    /// The session it names, and so creates.
    pub child: String,
    /// The literal prompt word, compared in memory and never kept.
    pub prompt: String,
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

/// Options that take exactly one value.
const VALUED: [&str; 4] = [
    "--model",
    "--effort",
    "--permission-mode",
    "--output-format",
];
/// Options that take none, other than `-p`.
const SWITCHES: [&str; 4] = [
    "--dangerously-skip-permissions",
    "--safe-mode",
    "--disable-slash-commands",
    "--strict-mcp-config",
];
/// The one MCP configuration accepted: no servers.
const EMPTY_MCP: &str = r#"{"mcpServers":{}}"#;

/// The launch a literal command submits, or `None` for anything else.
pub(super) fn launch(command: &str, programs: &[PathBuf]) -> Option<Launch> {
    let shell::Command {
        assignments: _,
        words,
        stdin,
        stdout: _,
    } = shell::command(command).ok()?;
    // The prompt is the one literal word; standard input is never read.
    if stdin.is_some() {
        return None;
    }
    let (program, args) = words.split_first()?;
    if !programs
        .iter()
        .any(|verified| verified.as_os_str() == program.as_str())
    {
        return None;
    }
    let mut print = 0;
    let mut json = false;
    let mut child: Option<&String> = None;
    let mut prompt: Option<&String> = None;
    let mut seen: Vec<&str> = Vec::new();
    let mut after_list = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let list_before = std::mem::take(&mut after_list);
        if !arg.starts_with('-') {
            // A plain word right after a list option's value would be read
            // as another value of it.
            if list_before || prompt.replace(arg).is_some() {
                return None;
            }
            continue;
        }
        let flag = arg.as_str();
        if flag == "-p" || flag == "--print" {
            print += 1;
            continue;
        }
        if seen.contains(&flag) {
            return None;
        }
        seen.push(flag);
        if SWITCHES.contains(&flag) {
            continue;
        }
        let value = args.next()?;
        match flag {
            "--session-id" if uuid(value) => child = Some(value),
            "--output-format" if value == "json" => json = true,
            "--mcp-config" if value == EMPTY_MCP => after_list = true,
            "--tools" | "--allowedTools" if !value.starts_with('-') => after_list = true,
            _ if VALUED.contains(&flag)
                && flag != "--output-format"
                && !value.is_empty()
                && !value.starts_with('-') => {}
            // `--tools`, `--allowedTools` and `--mcp-config` take several
            // values in the CLI: here exactly one, never followed by a plain
            // word. Resume, continue, fork, help, version, structured input,
            // `--`, `--flag=value` and every other option refuse.
            _ => return None,
        }
    }
    let prompt = prompt.filter(|prompt| !prompt.is_empty())?;
    if print != 1 || !json {
        return None;
    }
    Some(Launch {
        child: child?.clone(),
        prompt: prompt.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLAUDE: &str = "/opt/synthetic/bin/claude";
    const CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";

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
                prompt: "Synthetic prompt, it's literal".into(),
            })
        );
        assert_eq!(
            launch(
                &format!("{CLAUDE} --print 'p' --output-format json --session-id {CHILD}"),
                &programs()
            ),
            Some(Launch {
                child: CHILD.into(),
                prompt: "p".into()
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
            format!("{CLAUDE} -p --session-id={CHILD} --output-format json 'x'"),
            // Resume, continue, fork, help, version, structured input.
            format!("{base} --resume {CHILD} 'x'"),
            format!("{CLAUDE} -p --resume {CHILD} --output-format json 'x'"),
            format!("{base} -r {CHILD} 'x'"),
            format!("{base} --continue 'x'"),
            format!("{base} -c 'x'"),
            format!("{base} --fork-session 'x'"),
            format!("{base} --help"),
            format!("{base} --version"),
            format!("{base} --input-format stream-json 'x'"),
            format!("{base} -- 'x'"),
            // Output not JSON, print missing or repeated, no or two prompts.
            format!("{CLAUDE} -p --session-id {CHILD} --output-format text 'x'"),
            format!("{CLAUDE} --session-id {CHILD} --output-format json 'x'"),
            format!("{base} -p 'x'"),
            base.clone(),
            format!("{base} ''"),
            format!("{base} 'x' 'y'"),
            // Standard-input prompt, pipes, lists, substitution, background.
            format!("{base} < /w/brief.md"),
            format!("{base} 'x' < /w/brief.md"),
            format!("{base} 'x' | tee /w/o"),
            format!("{base} 'x'; true"),
            format!("{base} 'x' && true"),
            format!("{base} \"$(cat /w/brief)\""),
            format!("{base} 'x' &"),
            format!("{base} 'x' 2> /w/e"),
            // Unknown or repeated options, values that look like options.
            format!("{base} --agent a 'x'"),
            format!("{base} --model m --model n 'x'"),
            format!("{base} --model -x 'x'"),
            format!("{base} --safe-mode --safe-mode 'x'"),
            format!("{base} --mcp-config /w/mcp.json 'x'"),
            format!("{base} --mcp-config '{{\"mcpServers\":{{\"s\":{{}}}}}}' 'x'"),
            // A prompt that a list option would take as its second value.
            format!("{base} --tools Read 'x'"),
            format!("{base} --allowedTools Read 'x'"),
            format!("{base} --mcp-config '{EMPTY_MCP}' 'x'"),
            // Programs that are not a verified launcher.
            format!("claude -p --session-id {CHILD} --output-format json 'x'"),
            format!("/tmp/claude -p --session-id {CHILD} --output-format json 'x'"),
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
            prompt: "x".into(),
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
                prompt: "A=b".into()
            })
        );
        assert_eq!(
            launch(&format!("{base} A=b"), &programs()).map(|launch| launch.prompt),
            Some("A=b".into())
        );
        assert_eq!(launch(&format!("{base} 'x' A=b"), &programs()), None);
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
