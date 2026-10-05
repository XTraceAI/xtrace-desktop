//! A restricted reader for one literal shell command. It never runs or
//! expands anything: a command is a list of words made of plain characters,
//! single-quoted literals and the `'\''` idiom, separated by spaces, then at
//! most one standard-input and one standard-output redirection to a plain
//! absolute path, or it is refused. No other redirection, pipe, list,
//! substitution, variable, glob, tilde, `=word` expansion, double quote,
//! comment or newline is accepted.
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
/// is read from and its standard output is written to, if redirected. The
/// paths are words only; nothing here opens them.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Command {
    /// Literal assignments before the words.
    pub assignments: usize,
    pub words: Vec<String>,
    pub stdin: Option<String>,
    pub stdout: Option<String>,
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
        } => Ok(words),
        _ => Err(()),
    }
}

/// One simple command, or `Err`.
///
/// A redirection is exactly a standalone `<` or `>` after a space, then
/// spaces, then one unquoted word of plain characters that is an absolute
/// path outside `/dev`. Each appears at most once, and only after every
/// word. So `>>`, `>|`, `<<`, `<<<`, `<>`, `<&`, `&>`, `2>`, `>file`, a
/// device and a quoted, relative or expanded target are all refused.
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
            '<' | '>' => {
                // Standalone: a space before it (never the first character)
                // and a space after it.
                if word.is_some() || index == 0 || chars.get(index + 1) != Some(&' ') {
                    return Err(());
                }
                index += 1;
                while chars.get(index) == Some(&' ') {
                    index += 1;
                }
                let start = index;
                while index < chars.len() && chars[index] != ' ' {
                    if !plain(chars[index]) {
                        return Err(());
                    }
                    index += 1;
                }
                let path: String = chars[start..index].iter().collect();
                // Never a device: `< /dev/tty` reads what a person types.
                if !path.starts_with('/')
                    || path.split('/').any(|step| step == "." || step == "..")
                    || path.split('/').find(|step| !step.is_empty()) == Some("dev")
                {
                    return Err(());
                }
                let slot = if c == '<' { &mut stdin } else { &mut stdout };
                if slot.replace(path).is_some() {
                    return Err(());
                }
            }
            // Nothing but another redirection follows one.
            _ if stdin.is_some() || stdout.is_some() => return Err(()),
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
    })
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
    fn refuses_every_other_redirection() {
        for text in [
            "> /w/o",
            "< /w/i claude",
            "claude >/w/o",
            "claude>/w/o",
            "claude 'x'> /w/o",
            "claude >> /w/o",
            "claude >| /w/o",
            "claude 2> /w/o",
            "claude 2>&1",
            "claude > /w/o 2>&1",
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
            "'A'=b x", "A'='b x", "1A=b x", "A-B=b x", "A+=b x", "A.B=c x",
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
            "\"A\"=b x",
            "A=b",
            "A=b B=c",
            "A=b > /w/o x",
            "A=b; x",
        ] {
            assert!(command(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn refuses_anything_but_literal_words() {
        for command in [
            "",
            "   ",
            "claude -p \"$X\"",
            "claude -p \"$(cat /f)\"",
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
