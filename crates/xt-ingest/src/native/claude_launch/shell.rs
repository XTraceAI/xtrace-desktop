//! A restricted reader for one literal shell command. It never runs or
//! expands anything: a command is a list of words made of plain characters,
//! single-quoted literals, the `'\''` idiom and double-quoted literals holding
//! no `$`, backquote, backslash or newline, separated by spaces, then at most
//! one standard-input and one standard-output redirection to a plain absolute
//! path and one standard-error redirection to such a path or to standard
//! output (`2>&1`), or it is refused. No other redirection, pipe, list,
//! substitution, variable, glob, tilde, `=word` expansion, other double
//! quote, comment or newline is accepted.
//!
//! Leading words that are literal assignments — an unquoted name
//! (`[A-Za-z_][A-Za-z0-9_]*`) then an unquoted `=`, then a value of the same
//! literal characters and quoting, possibly empty, not starting with an
//! unquoted `=` — are told apart before quoting is lost, counted and set
//! aside: the command's words start at the first word that is not one. A
//! quoted name or `=`, another name, `+=`, an array and every expansion
//! remain refused or ordinary words.

fn plain(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '_' | '.' | '/' | ':' | '=' | ',' | '+' | '@' | '%' | '-')
}

/// One simple command: its literal words, then the paths its standard input
/// is read from and its standard output and error are written to, if
/// redirected. The paths are words only; nothing here opens them.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Command {
    /// Literal assignments before the words.
    pub assignments: usize,
    pub words: Vec<String>,
    pub stdin: Option<String>,
    pub stdout: Option<String>,
    pub stderr: Option<Stderr>,
}

/// Where a redirected standard error goes.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Stderr {
    File(String),
    /// `2>&1`.
    Stdout,
}

/// The literal words of one simple command, or `Err`. A redirection is
/// refused.
#[cfg(test)]
pub(super) fn words(command: &str) -> Result<Vec<String>, ()> {
    match self::command(command)? {
        Command {
            assignments: 0,
            words,
            stdin: None,
            stdout: None,
            stderr: None,
        } => Ok(words),
        _ => Err(()),
    }
}

/// One simple command, or `Err`.
///
/// A redirection is exactly a standalone `<`, `>` or `2>` after a space,
/// then spaces, then one unquoted word of plain characters that is an
/// absolute path outside `/dev`; or a standalone `2>&1`. Standard input,
/// output and error are each redirected at most once, and only after every
/// word. So `>>`, `>|`, `<<`, `<<<`, `<>`, `<&`, `&>`, `2>>`, `2>&2`, `1>`,
/// `>file`, `2>file`, a device and a quoted, relative or expanded target are
/// all refused.
pub(super) fn command(command: &str) -> Result<Command, ()> {
    let chars: Vec<char> = command.chars().collect();
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut assignments = 0;
    // The current word: still only an unquoted name, so far; an assignment
    // since its unquoted `=`; its value still empty.
    let (mut name, mut assignment, mut value_empty) = (false, false, false);
    let mut stdin: Option<String> = None;
    let mut stdout: Option<String> = None;
    let mut stderr: Option<Stderr> = None;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        match c {
            ' ' => {
                if let Some(done) = word.take() {
                    if assignment {
                        assignments += 1;
                    } else {
                        words.push(done);
                    }
                }
                assignment = false;
                index += 1;
            }
            // `2>&1` or `2>`, standalone after a space.
            '2' if word.is_none() && index > 0 && chars.get(index + 1) == Some(&'>') => {
                if stderr.is_some() {
                    return Err(());
                }
                if chars.get(index + 2) == Some(&'&') {
                    if chars.get(index + 3) != Some(&'1')
                        || chars.get(index + 4).is_some_and(|&next| next != ' ')
                    {
                        return Err(());
                    }
                    stderr = Some(Stderr::Stdout);
                    index += 4;
                    continue;
                }
                if chars.get(index + 2) != Some(&' ') {
                    return Err(());
                }
                index += 2;
                stderr = Some(Stderr::File(target(&chars, &mut index)?));
            }
            '<' | '>' => {
                // Standalone: a space before it (never the first character)
                // and a space after it.
                if word.is_some() || index == 0 || chars.get(index + 1) != Some(&' ') {
                    return Err(());
                }
                index += 1;
                let path = target(&chars, &mut index)?;
                let slot = if c == '<' { &mut stdin } else { &mut stdout };
                if slot.replace(path).is_some() {
                    return Err(());
                }
            }
            // Nothing but another redirection follows one.
            _ if stdin.is_some() || stdout.is_some() || stderr.is_some() => return Err(()),
            '\'' => {
                let end = chars[index + 1..]
                    .iter()
                    .position(|&c| c == '\'')
                    .ok_or(())?;
                word.get_or_insert_with(String::new)
                    .extend(&chars[index + 1..index + 1 + end]);
                (name, value_empty) = (false, false);
                index += end + 2;
            }
            // A double-quoted literal: nothing in it a shell would expand or
            // escape, so it reads exactly as written.
            '"' => {
                let end = chars[index + 1..]
                    .iter()
                    .position(|&c| c == '"')
                    .ok_or(())?;
                let text = &chars[index + 1..index + 1 + end];
                if text.iter().any(|c| matches!(c, '$' | '`' | '\\' | '\n')) {
                    return Err(());
                }
                word.get_or_insert_with(String::new).extend(text);
                (name, value_empty) = (false, false);
                index += end + 2;
            }
            '\\' => {
                if chars.get(index + 1) != Some(&'\'') {
                    return Err(());
                }
                word.get_or_insert_with(String::new).push('\'');
                (name, value_empty) = (false, false);
                index += 2;
            }
            // zsh expands an unquoted word, or an assigned value, that
            // starts with `=`.
            '=' if word.is_none() || (assignment && value_empty) => return Err(()),
            // An unquoted name then an unquoted `=`, while only assignments
            // came before: a literal assignment.
            '=' if name && words.is_empty() && !assignment => {
                word.get_or_insert_with(String::new).push(c);
                (name, assignment, value_empty) = (false, true, true);
                index += 1;
            }
            _ if plain(c) => {
                let fresh = word.is_none();
                word.get_or_insert_with(String::new).push(c);
                name = if fresh {
                    c.is_ascii_alphabetic() || c == '_'
                } else {
                    name && (c.is_ascii_alphanumeric() || c == '_')
                };
                value_empty = false;
                index += 1;
            }
            _ => return Err(()),
        }
    }
    if let Some(done) = word {
        if assignment {
            assignments += 1;
        } else {
            words.push(done);
        }
    }
    if words.is_empty() || words.iter().any(|word| word.contains('\0')) {
        return Err(());
    }
    Ok(Command {
        assignments,
        words,
        stdin,
        stdout,
        stderr,
    })
}

/// A redirection's target from `index`, past the spaces after its operator:
/// one unquoted word of plain characters that is an absolute path outside
/// `/dev`.
fn target(chars: &[char], index: &mut usize) -> Result<String, ()> {
    while chars.get(*index) == Some(&' ') {
        *index += 1;
    }
    let start = *index;
    while *index < chars.len() && chars[*index] != ' ' {
        if !plain(chars[*index]) {
            return Err(());
        }
        *index += 1;
    }
    let path: String = chars[start..*index].iter().collect();
    // Never a device: `< /dev/tty` reads what a person types.
    if !path.starts_with('/')
        || path.split('/').any(|step| step == "." || step == "..")
        || path.split('/').find(|step| !step.is_empty()) == Some("dev")
    {
        return Err(());
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_plain_and_single_quoted_words() {
        assert_eq!(
            words("/opt/claude -p --model m 'it'\\''s \"q\"\n$HOME' --output-format json").unwrap(),
            [
                "/opt/claude",
                "-p",
                "--model",
                "m",
                "it's \"q\"\n$HOME",
                "--output-format",
                "json"
            ]
        );
        assert_eq!(words("a ''").unwrap(), ["a", ""]);
    }

    #[test]
    fn reads_one_trailing_stdin_and_stdout_redirection() {
        assert_eq!(
            command("/opt/claude -p 'x y' > /w/o.json").unwrap(),
            Command {
                assignments: 0,
                words: vec!["/opt/claude".into(), "-p".into(), "x y".into()],
                stdin: None,
                stdout: Some("/w/o.json".into()),
                stderr: None,
            }
        );
        for text in [
            "/opt/claude -p < /w/brief.md > /w/o.json",
            "/opt/claude -p > /w/o.json < /w/brief.md",
            "/opt/claude -p  <  /w/brief.md  >  /w/o.json ",
        ] {
            assert_eq!(
                command(text).unwrap(),
                Command {
                    assignments: 0,
                    words: vec!["/opt/claude".into(), "-p".into()],
                    stdin: Some("/w/brief.md".into()),
                    stdout: Some("/w/o.json".into()),
                    stderr: None,
                },
                "{text:?}"
            );
        }
        // `2 >` is the word `2`, then a redirection of standard output.
        assert_eq!(
            command("/opt/claude 2 > /w/o").unwrap().words,
            ["/opt/claude", "2"]
        );
    }

    #[test]
    fn reads_one_standard_error_redirection_to_a_file_or_standard_output() {
        let file = || Some(Stderr::File("/w/e.log".into()));
        for (text, stdin, stdout, stderr) in [
            ("/opt/claude -p x 2> /w/e.log", None, None, file()),
            ("/opt/claude -p x 2>  /w/e.log ", None, None, file()),
            ("/opt/claude -p x 2>&1", None, None, Some(Stderr::Stdout)),
            (
                "/opt/claude -p < /w/i > /w/o 2> /w/e.log",
                Some("/w/i"),
                Some("/w/o"),
                file(),
            ),
            (
                "/opt/claude -p 2> /w/e.log < /w/i > /w/o",
                Some("/w/i"),
                Some("/w/o"),
                file(),
            ),
            (
                "/opt/claude -p x > /w/o 2>&1",
                None,
                Some("/w/o"),
                Some(Stderr::Stdout),
            ),
            (
                "/opt/claude -p x 2>&1 > /w/o",
                None,
                Some("/w/o"),
                Some(Stderr::Stdout),
            ),
        ] {
            let read = command(text).unwrap();
            assert_eq!(read.stdin.as_deref(), stdin, "{text:?}");
            assert_eq!(read.stdout.as_deref(), stdout, "{text:?}");
            assert_eq!(read.stderr, stderr, "{text:?}");
            assert_eq!(
                read.words.last().map(String::as_str),
                Some(if stdin.is_some() { "-p" } else { "x" })
            );
        }
        for text in [
            "claude 2> /w/e 2> /w/f",
            "claude 2>&1 2>&1",
            "claude 2> /w/e 2>&1",
            "claude 2>> /w/e",
            "claude 2>/w/e",
            "claude 2>&2",
            "claude 2>&1x",
            "claude 2>& 1",
            "claude 2> /dev/null",
            "claude 2> w/e",
            "claude 2> '/w/e'",
            "claude x2> /w/e",
            "claude 1> /w/o",
            "claude 2> /w/e x",
            "claude 2>&1 x",
            "claude 2>&1 | tee /w/o",
            "2> /w/e claude",
        ] {
            assert!(command(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn refuses_every_other_redirection() {
        for text in [
            "> /w/o",
            "< /w/i claude",
            "claude >/w/o",
            "claude>/w/o",
            "claude 'x'> /w/o",
            "claude >> /w/o",
            "claude >| /w/o",
            "claude &> /w/o",
            "claude > /w/o > /w/p",
            "claude < /w/i < /w/j",
            "claude << EOF",
            "claude <<< x",
            "claude <> /w/o",
            "claude <& 0",
            "claude > w/o",
            "claude > ./o",
            "claude > ~/o",
            "claude > /w/../o",
            "claude > /w/./o",
            "claude > '/w/o'",
            "claude > /w/$X",
            "claude > /w/*.json",
            "claude > /dev/stdout",
            "claude < /dev/tty",
            "claude < //dev/tty",
            "claude > /w/o x",
            "claude > /w/o 'x'",
            "claude > /w/o | tee /w/p",
            "claude > /w/o; true",
            "claude > /w/o &",
            "claude >",
            "claude > ",
        ] {
            assert!(command(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn sets_leading_literal_assignments_aside() {
        for (text, assignments, first) in [
            ("A=b /opt/claude -p", 1, "/opt/claude"),
            (
                "_x9=/w/a:/w/b B= C='' D='x y' E='it'\\''s' /opt/claude",
                5,
                "/opt/claude",
            ),
            ("A=b=c /opt/claude", 1, "/opt/claude"),
            ("/opt/claude A=b", 0, "/opt/claude"),
        ] {
            let read = command(text).unwrap();
            assert_eq!(
                (read.assignments, read.words[0].as_str()),
                (assignments, first),
                "{text:?}"
            );
        }
        assert_eq!(
            command("/opt/claude A=b").unwrap().words,
            ["/opt/claude", "A=b"]
        );
        // Not an assignment: a quoted name or `=`, another name, `+=`.
        for text in [
            "'A'=b x",
            "\"A\"=b x",
            "A'='b x",
            "1A=b x",
            "A-B=b x",
            "A+=b x",
            "A.B=c x",
        ] {
            let read = command(text).unwrap();
            assert_eq!(read.assignments, 0, "{text:?}");
            assert_eq!(read.words.len(), 2, "{text:?}");
        }
        // Refused: expansions, arrays, a value starting with `=`, nothing
        // but assignments, or an assignment after a redirection.
        for text in [
            "A=$B x",
            "A=`b` x",
            "A=~/b x",
            "A=(b) x",
            "A[1]=b x",
            "A==b x",
            "A=b",
            "A=b B=c",
            "A=b > /w/o x",
            "A=b; x",
        ] {
            assert!(command(text).is_err(), "{text:?}");
        }
    }

    /// A double-quoted literal reads as written, alone or joined to other
    /// quoting in one word.
    #[test]
    fn reads_literal_double_quotes() {
        assert_eq!(
            words("claude --name \"Fix active session names\" -p x").unwrap(),
            ["claude", "--name", "Fix active session names", "-p", "x"]
        );
        assert_eq!(
            words("a\"b c\"'d e'\"\" \"it's\"").unwrap(),
            ["ab cd e", "it's"]
        );
    }

    #[test]
    fn refuses_anything_but_literal_words() {
        for command in [
            "",
            "   ",
            "claude -p \"$X\"",
            "claude -p \"$(cat /f)\"",
            "claude -p \"a`b`\"",
            "claude -p \"a\\\"b\"",
            "claude -p \"a\\nb\"",
            "claude -p \"a\nb\"",
            "claude -p \"open",
            "claude -p $(cat /f)",
            "claude -p `cat f`",
            "claude -p x > /tmp/o.json",
            "claude -p x >/tmp/o.json",
            "claude -p x 2> /tmp/e",
            "claude -p x < /tmp/in",
            "claude -p x >> /tmp/o",
            "claude -p x &> /tmp/o",
            "claude -p x | tee y",
            "claude -p x; rm y",
            "claude -p x && y",
            "claude -p x &",
            "claude -p x\nrm y",
            "claude -p x # c",
            "claude -p ~/x",
            "claude -p *.json",
            "claude -p {a,b}",
            "claude -p =ls",
            "claude -p 'x",
            "claude -p 'x\0'",
            "claude -p x\\ y",
            "(claude -p x)",
            "claude\t-p x",
        ] {
            assert!(words(command).is_err(), "{command:?}");
        }
    }
}
