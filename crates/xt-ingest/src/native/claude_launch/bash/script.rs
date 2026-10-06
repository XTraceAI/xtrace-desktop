//! A finite reader for one `Bash` call's command that launches Claude print
//! sessions. It never runs or expands anything itself: it reads a closed
//! grammar, renders the few variables the grammar allows exactly as a shell
//! would, and refuses everything else.
//!
//! ```text
//! script := stmt (';' stmt)* [';']
//! stmt   := NAME=word | for NAME in word...; do unit (; unit)* [;] done | unit
//! unit   := echo word... | ( cd word && launch )
//! launch := PROGRAM arg... [2>&1] [| tail -N | tail -n N]
//! ```
//!
//! A word is plain characters, single-quoted text, and double-quoted text in
//! which `$NAME` and `${NAME}` name a variable this script set earlier (an
//! assignment or the loop's own variable) and `\` escapes only `$`, `` ` ``,
//! `"` and `\`. An unquoted expansion, a command or arithmetic substitution,
//! a backtick, `$'…'`, a glob, a tilde, a brace, a comment, an unquoted
//! newline, a nested loop, a background job, a list other than the one
//! `cd … &&` form, any other redirection or pipe, a function and an unknown
//! variable are all refused, and so is an assignment to a name that steers
//! where or how a program runs (`PATH`, `HOME`, `CLAUDE_…` and the like).
//! Loop words are literal. A rendered `echo` line holds no backslash and does
//! not start with `-`; a directory is absolute with no `.` or `..` step.
//!
//! The program is `claude` as the shell finds it — taken on the local
//! history's word, since no record says which program that name ran then —
//! or one of the known installed launchers by its absolute path. Its options
//! are read by the shared option map ([`options`]) that the Codex launch
//! reader also uses: any known option in any order, one prompt word, and
//! only a fresh print session ([`Action::Create`]) is a launch. Resume,
//! continue, fork, background, no saved session, help, version, subcommands,
//! structured input and every unknown option refuse the command. What the
//! launch prints is kept as read ([`Launch::format`], [`Launch::verbose`]):
//! only a plain answer or one JSON result can be matched later, so a
//! verbose launch (streaming JSON needs one) is recognized but decides
//! nothing.
//!
//! What the call printed is then split, exactly, into the echo lines and
//! each launch's complete output; see [`partition`].

use super::super::options::{self, Action, Format};
use std::{collections::HashMap, path::PathBuf};

/// Longest command read.
pub(super) const MAX_COMMAND: usize = 64 * 1024;
/// Statements run, counting each loop body once per word.
const MAX_STEPS: usize = 64;
/// Words one loop runs over.
const MAX_LOOP_WORDS: usize = 16;
/// Launches one command runs.
pub(super) const MAX_LAUNCHES: usize = 16;
/// Longest rendered word.
const MAX_WORD: usize = 64 * 1024;

/// One rendered launch: where it ran and what it submitted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Launch {
    pub cwd: String,
    pub prompt: String,
    /// The session the command named with `--session-id`.
    pub session_id: Option<String>,
    /// What the launch prints: its plain answer, one JSON result or
    /// streaming JSON events.
    pub format: Format,
    /// Verbose logging, which changes what is printed.
    pub verbose: bool,
    /// The last lines `tail` kept, when piped through it.
    pub tail: Option<usize>,
}

/// What the script prints, in order: echo lines and launch outputs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Item {
    Echo(String),
    Launch(usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Script {
    pub items: Vec<Item>,
    pub launches: Vec<Launch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Part {
    Lit(String),
    Var(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Word {
    parts: Vec<Part>,
    /// The leading unquoted characters, as typed.
    bare: String,
    /// Whether everything is that unquoted text.
    plain: bool,
}

impl Word {
    fn new() -> Self {
        Self {
            parts: Vec::new(),
            bare: String::new(),
            plain: true,
        }
    }

    fn push_lit(&mut self, c: char, quoted: bool) {
        if quoted {
            self.plain = false;
        } else if self.plain {
            self.bare.push(c);
        }
        match self.parts.last_mut() {
            Some(Part::Lit(text)) => text.push(c),
            _ => self.parts.push(Part::Lit(c.to_string())),
        }
    }

    fn keyword(&self) -> Option<&str> {
        self.plain.then_some(self.bare.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Word(Word),
    Semi,
    Open,
    Close,
    And,
    Pipe,
    /// `2>&1`.
    Merge,
}

fn plain(c: char) -> bool {
    c.is_ascii_alphanumeric()
        || matches!(c, '_' | '.' | '/' | ':' | '=' | ',' | '+' | '@' | '%' | '-')
}

fn name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn tokens(command: &str) -> Option<Vec<Tok>> {
    let chars: Vec<char> = command.chars().collect();
    let mut out = Vec::new();
    let mut word: Option<Word> = None;
    let mut i = 0;
    let end = |word: &mut Option<Word>, out: &mut Vec<Tok>| {
        if let Some(done) = word.take() {
            out.push(Tok::Word(done));
        }
    };
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' => {
                end(&mut word, &mut out);
                i += 1;
            }
            ';' => {
                end(&mut word, &mut out);
                out.push(Tok::Semi);
                i += 1;
            }
            '(' => {
                if word.is_some() {
                    return None;
                }
                out.push(Tok::Open);
                i += 1;
            }
            ')' => {
                end(&mut word, &mut out);
                out.push(Tok::Close);
                i += 1;
            }
            '\\' => {
                // One escaped character, as in the `'\''` idiom.
                let next = *chars.get(i + 1)?;
                if next == '\n' {
                    return None;
                }
                word.get_or_insert_with(Word::new).push_lit(next, true);
                i += 2;
            }
            '&' => {
                if chars.get(i + 1) != Some(&'&') {
                    return None;
                }
                end(&mut word, &mut out);
                out.push(Tok::And);
                i += 2;
            }
            '|' => {
                if chars.get(i + 1) == Some(&'|') || chars.get(i + 1) == Some(&'&') {
                    return None;
                }
                end(&mut word, &mut out);
                out.push(Tok::Pipe);
                i += 1;
            }
            '>' => {
                // Only `2>&1`, standing alone.
                let merge = word.as_ref().and_then(Word::keyword) == Some("2")
                    && chars.get(i + 1) == Some(&'&')
                    && chars.get(i + 2) == Some(&'1')
                    && chars
                        .get(i + 3)
                        .is_none_or(|next| matches!(next, ' ' | '\t' | ';' | ')' | '|'));
                if !merge {
                    return None;
                }
                word = None;
                out.push(Tok::Merge);
                i += 3;
            }
            '\'' => {
                let close = chars[i + 1..].iter().position(|&c| c == '\'')?;
                let current = word.get_or_insert_with(Word::new);
                current.plain = false;
                if close == 0 && !matches!(current.parts.last(), Some(Part::Lit(_))) {
                    current.parts.push(Part::Lit(String::new()));
                }
                for &c in &chars[i + 1..i + 1 + close] {
                    current.push_lit(c, true);
                }
                i += close + 2;
            }
            '"' => {
                let current = word.get_or_insert_with(Word::new);
                current.plain = false;
                if !matches!(current.parts.last(), Some(Part::Lit(_))) {
                    current.parts.push(Part::Lit(String::new()));
                }
                i += 1;
                loop {
                    let c = *chars.get(i)?;
                    match c {
                        '"' => {
                            i += 1;
                            break;
                        }
                        '`' => return None,
                        '\\' => {
                            let next = *chars.get(i + 1)?;
                            if matches!(next, '$' | '`' | '"' | '\\') {
                                current.push_lit(next, true);
                            } else if next == '\n' {
                                return None;
                            } else {
                                current.push_lit('\\', true);
                                current.push_lit(next, true);
                            }
                            i += 2;
                        }
                        '$' => {
                            let (name, used) = match chars.get(i + 1) {
                                Some('{') => {
                                    let close = chars[i + 2..].iter().position(|&c| c == '}')?;
                                    let name: String = chars[i + 2..i + 2 + close].iter().collect();
                                    (name, close + 3)
                                }
                                Some(&first) if name_start(first) => {
                                    let len = chars[i + 1..]
                                        .iter()
                                        .take_while(|&&c| name_char(c))
                                        .count();
                                    (chars[i + 1..i + 1 + len].iter().collect(), len + 1)
                                }
                                _ => return None,
                            };
                            let mut letters = name.chars();
                            if !letters.next().is_some_and(name_start) || !letters.all(name_char) {
                                return None;
                            }
                            current.parts.push(Part::Var(name));
                            i += used;
                        }
                        _ => {
                            current.push_lit(c, true);
                            i += 1;
                        }
                    }
                }
            }
            _ if plain(c) => {
                word.get_or_insert_with(Word::new).push_lit(c, false);
                i += 1;
            }
            _ => return None,
        }
    }
    end(&mut word, &mut out);
    Some(out)
}

#[derive(Debug)]
enum Unit {
    Echo(Vec<Word>),
    Launch {
        cwd: Word,
        program: Word,
        args: Vec<Word>,
        tail: Option<usize>,
    },
}

#[derive(Debug)]
enum Stmt {
    Assign(String, Word),
    Loop(String, Vec<Word>, Vec<Unit>),
    Unit(Unit),
}

/// Names whose assignment changes which program runs, where it keeps its
/// history or how the shell reads words.
fn steering(name: &str) -> bool {
    matches!(
        name,
        "PATH"
            | "HOME"
            | "IFS"
            | "CDPATH"
            | "ENV"
            | "BASH_ENV"
            | "ZDOTDIR"
            | "SHELL"
            | "PWD"
            | "OLDPWD"
            | "TMPDIR"
            | "XDG_CONFIG_HOME"
            | "XDG_DATA_HOME"
    ) || ["CLAUDE", "ANTHROPIC", "NODE", "DYLD_", "LD_", "BUN", "NPM"]
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// A literal assignment word: an unquoted name, `=`, then any value.
fn assignment(word: &Word) -> Option<(String, Word)> {
    let first = match word.parts.first()? {
        Part::Lit(text) => text,
        Part::Var(_) => return None,
    };
    let eq = word.bare.find('=')?;
    let name = &word.bare[..eq];
    let mut letters = name.chars();
    if !letters.next().is_some_and(name_start) || !letters.all(name_char) || steering(name) {
        return None;
    }
    let mut parts = word.parts.clone();
    let rest = first[eq + 1..].to_owned();
    if rest.is_empty() && parts.len() > 1 {
        parts.remove(0);
    } else {
        parts[0] = Part::Lit(rest);
    }
    Some((
        name.to_owned(),
        Word {
            parts,
            bare: String::new(),
            plain: false,
        },
    ))
}

struct Parser {
    toks: Vec<Tok>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.at)
    }

    fn next(&mut self) -> Option<Tok> {
        let tok = self.toks.get(self.at).cloned();
        self.at += 1;
        tok
    }

    fn keyword(&self, expected: &str) -> bool {
        matches!(self.peek(), Some(Tok::Word(word)) if word.keyword() == Some(expected))
    }

    fn expect(&mut self, expected: &str) -> Option<()> {
        self.keyword(expected).then(|| self.at += 1)
    }

    fn word(&mut self) -> Option<Word> {
        match self.next()? {
            Tok::Word(word) => Some(word),
            _ => None,
        }
    }

    fn script(&mut self) -> Option<Vec<Stmt>> {
        let mut stmts = Vec::new();
        while self.peek().is_some() {
            stmts.push(self.stmt()?);
            match self.next() {
                None => break,
                Some(Tok::Semi) => {}
                Some(_) => return None,
            }
        }
        Some(stmts)
    }

    fn stmt(&mut self) -> Option<Stmt> {
        if self.keyword("for") {
            self.at += 1;
            let name = self.word()?;
            let name = name.keyword()?.to_owned();
            let mut letters = name.chars();
            if !letters.next().is_some_and(name_start) || !letters.all(name_char) || steering(&name)
            {
                return None;
            }
            self.expect("in")?;
            let mut words = Vec::new();
            while let Some(Tok::Word(_)) = self.peek() {
                words.push(self.word()?);
            }
            if words.is_empty() || words.len() > MAX_LOOP_WORDS {
                return None;
            }
            matches!(self.next()?, Tok::Semi).then_some(())?;
            self.expect("do")?;
            let mut body = vec![self.unit()?];
            loop {
                if self.keyword("done") {
                    self.at += 1;
                    break;
                }
                matches!(self.next()?, Tok::Semi).then_some(())?;
                if self.keyword("done") {
                    self.at += 1;
                    break;
                }
                body.push(self.unit()?);
            }
            return Some(Stmt::Loop(name, words, body));
        }
        if let Some(Tok::Word(word)) = self.peek()
            && let Some((name, value)) = assignment(word)
        {
            self.at += 1;
            // A lone assignment, not a prefix of a command.
            return matches!(self.peek(), None | Some(Tok::Semi))
                .then_some(Stmt::Assign(name, value));
        }
        Some(Stmt::Unit(self.unit()?))
    }

    fn unit(&mut self) -> Option<Unit> {
        if self.keyword("echo") {
            self.at += 1;
            let mut words = Vec::new();
            while let Some(Tok::Word(_)) = self.peek() {
                words.push(self.word()?);
            }
            return (!words.is_empty()).then_some(Unit::Echo(words));
        }
        matches!(self.next()?, Tok::Open).then_some(())?;
        self.expect("cd")?;
        let cwd = self.word()?;
        matches!(self.next()?, Tok::And).then_some(())?;
        let program = self.word()?;
        let mut args = Vec::new();
        while let Some(Tok::Word(_)) = self.peek() {
            args.push(self.word()?);
        }
        if self.peek() == Some(&Tok::Merge) {
            self.at += 1;
        }
        let mut tail = None;
        if self.peek() == Some(&Tok::Pipe) {
            self.at += 1;
            self.expect("tail")?;
            let count = self.word()?;
            let count = match count.keyword()? {
                "-n" => self.word()?.keyword()?.to_owned(),
                flag => flag.strip_prefix('-')?.to_owned(),
            };
            if count.is_empty() || !count.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            tail = Some(count.parse::<usize>().ok().filter(|&n| n > 0)?);
        }
        matches!(self.next()?, Tok::Close).then_some(())?;
        Some(Unit::Launch {
            cwd,
            program,
            args,
            tail,
        })
    }
}

fn render(word: &Word, vars: &HashMap<String, String>) -> Option<String> {
    let mut out = String::new();
    for part in &word.parts {
        match part {
            Part::Lit(text) => out.push_str(text),
            Part::Var(name) => out.push_str(vars.get(name)?),
        }
        if out.len() > MAX_WORD {
            return None;
        }
    }
    Some(out)
}

/// A literal word: no variable at all.
fn literal(word: &Word) -> Option<String> {
    word.parts
        .iter()
        .all(|part| matches!(part, Part::Lit(_)))
        .then(|| render(word, &HashMap::new()))?
}

fn launch(
    cwd: String,
    program: &Word,
    args: &[String],
    tail: Option<usize>,
    programs: &[PathBuf],
) -> Option<Launch> {
    let program = literal(program)?;
    if program != "claude"
        && !programs
            .iter()
            .any(|known| known.as_os_str() == program.as_str())
    {
        return None;
    }
    let read = options::read(args)?;
    if read.action != Action::Create {
        return None;
    }
    let prompt = read.prompt.filter(|prompt| !prompt.is_empty())?;
    if !cwd.starts_with('/')
        || cwd.contains('\n')
        || cwd.split('/').any(|step| step == "." || step == "..")
    {
        return None;
    }
    Some(Launch {
        cwd,
        prompt,
        session_id: read.session_id,
        format: read.format,
        verbose: read.verbose,
        tail,
    })
}

struct Run<'a> {
    vars: HashMap<String, String>,
    items: Vec<Item>,
    launches: Vec<Launch>,
    steps: usize,
    programs: &'a [PathBuf],
}

impl Run<'_> {
    fn unit(&mut self, unit: &Unit) -> Option<()> {
        self.steps += 1;
        if self.steps > MAX_STEPS {
            return None;
        }
        match unit {
            Unit::Echo(words) => {
                let rendered = words
                    .iter()
                    .map(|word| render(word, &self.vars))
                    .collect::<Option<Vec<_>>>()?;
                let line = rendered.join(" ");
                if rendered[0].starts_with('-') || line.contains('\\') || line.contains('\n') {
                    return None;
                }
                self.items.push(Item::Echo(line));
            }
            Unit::Launch {
                cwd,
                program,
                args,
                tail,
            } => {
                // Two launches with nothing printed between them cannot be
                // told apart.
                if matches!(self.items.last(), Some(Item::Launch(_)))
                    || self.launches.len() >= MAX_LAUNCHES
                {
                    return None;
                }
                let cwd = render(cwd, &self.vars)?;
                let args = args
                    .iter()
                    .map(|word| render(word, &self.vars))
                    .collect::<Option<Vec<_>>>()?;
                let launch = launch(cwd, program, &args, *tail, self.programs)?;
                self.items.push(Item::Launch(self.launches.len()));
                self.launches.push(launch);
            }
        }
        Some(())
    }
}

/// The launches a command runs, in the order it runs them, and what it
/// prints; `None` for anything outside the grammar or without a launch.
pub(super) fn parse(command: &str, programs: &[PathBuf]) -> Option<Script> {
    if command.len() > MAX_COMMAND {
        return None;
    }
    let mut parser = Parser {
        toks: tokens(command)?,
        at: 0,
    };
    let stmts = parser.script()?;
    let mut run = Run {
        vars: HashMap::new(),
        items: Vec::new(),
        launches: Vec::new(),
        steps: 0,
        programs,
    };
    for stmt in &stmts {
        match stmt {
            Stmt::Assign(name, value) => {
                run.steps += 1;
                let value = render(value, &run.vars)?;
                run.vars.insert(name.clone(), value);
            }
            Stmt::Loop(name, words, body) => {
                let words = words.iter().map(literal).collect::<Option<Vec<_>>>()?;
                for word in words {
                    run.vars.insert(name.clone(), word);
                    for unit in body {
                        run.unit(unit)?;
                    }
                }
            }
            Stmt::Unit(unit) => run.unit(unit)?,
        }
        if run.steps > MAX_STEPS {
            return None;
        }
    }
    if run.launches.is_empty() {
        return None;
    }
    Some(Script {
        items: run.items,
        launches: run.launches,
    })
}

/// Each launch's complete output in what the call printed, with its final
/// newline removed, or `None`.
///
/// The printed text is exactly the echo lines and the launches' outputs in
/// order, every one ending in a newline; a result may lack only the very
/// last newline. Each launch output runs up to the next echo line and is at
/// least one line; no line of it may equal any echo line of the script, and
/// nothing may be left over. A launch through `tail -N` prints at most N
/// lines.
pub(super) fn partition<'a>(script: &Script, printed: &'a str) -> Option<Vec<&'a str>> {
    let text_end = printed.len();
    let mut lines: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    while start < text_end {
        let end = printed[start..]
            .find('\n')
            .map_or(text_end, |at| start + at + 1);
        lines.push((start, end));
        start = end;
    }
    let line = |index: usize| {
        let (start, end) = lines[index];
        printed[start..end]
            .strip_suffix('\n')
            .unwrap_or(&printed[start..end])
    };
    let markers: Vec<&str> = script
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Echo(text) => Some(text.as_str()),
            Item::Launch(_) => None,
        })
        .collect();
    let mut chunks = vec![""; script.launches.len()];
    let mut at = 0;
    for (position, item) in script.items.iter().enumerate() {
        match item {
            Item::Echo(text) => {
                if at >= lines.len() || line(at) != text {
                    return None;
                }
                at += 1;
            }
            Item::Launch(index) => {
                let next = match script.items.get(position + 1) {
                    Some(Item::Echo(text)) => Some(text.as_str()),
                    _ => None,
                };
                let first = at;
                while at < lines.len() && Some(line(at)) != next {
                    if markers.contains(&line(at)) {
                        return None;
                    }
                    at += 1;
                }
                if at == first {
                    return None;
                }
                let launch = &script.launches[*index];
                if launch.tail.is_some_and(|tail| at - first > tail) {
                    return None;
                }
                let (from, _) = lines[first];
                let (_, to) = lines[at - 1];
                let chunk = &printed[from..to];
                chunks[*index] = chunk.strip_suffix('\n').unwrap_or(chunk);
            }
        }
    }
    (at == lines.len()).then_some(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOP: &str = r#"Q="Synthetic question?"; for d in /work/alpha /work/beta; do echo "== started in $d"; (cd "$d" && claude -p "$Q" --max-turns 1 2>&1 | tail -4); done"#;

    fn launches(command: &str) -> Option<Vec<Launch>> {
        parse(command, &[]).map(|script| script.launches)
    }

    #[test]
    fn a_loop_runs_one_launch_per_word_in_order() {
        let script = parse(LOOP, &[]).unwrap();
        assert_eq!(
            script.items,
            vec![
                Item::Echo("== started in /work/alpha".into()),
                Item::Launch(0),
                Item::Echo("== started in /work/beta".into()),
                Item::Launch(1),
            ]
        );
        let expected = |cwd: &str| Launch {
            cwd: cwd.into(),
            prompt: "Synthetic question?".into(),
            session_id: None,
            format: Format::Text,
            verbose: false,
            tail: Some(4),
        };
        assert_eq!(
            script.launches,
            vec![expected("/work/alpha"), expected("/work/beta")]
        );
    }

    #[test]
    fn names_values_markers_and_options_are_arbitrary_within_the_grammar() {
        let command = r#"ASK='It'\''s "quoted"'; for place in /p/one /p/two /p/three; do echo "=> ${place} <=" now; ( cd "${place}" && claude --model m1 --print "$ASK" --output-format json --session-id 0c000000-0000-4000-8000-0000000000c1 ); done;"#;
        let script = parse(command, &[]).unwrap();
        assert_eq!(script.launches.len(), 3);
        assert_eq!(script.launches[2].cwd, "/p/three");
        assert_eq!(script.launches[0].prompt, "It's \"quoted\"");
        assert_eq!(script.launches[0].format, Format::Json);
        assert_eq!(
            script.items[0],
            Item::Echo("=> /p/one <= now".into()),
            "{script:?}"
        );
        // One launch alone, no marker needed; an installed launcher path.
        let installed = [PathBuf::from("/opt/homebrew/bin/claude")];
        let single = parse(
            "(cd /p/one && /opt/homebrew/bin/claude -p 'Hello')",
            &installed,
        )
        .unwrap();
        assert_eq!(single.items, vec![Item::Launch(0)]);
        assert!(parse("(cd /p/one && /tmp/claude -p 'Hello')", &installed).is_none());
    }

    #[test]
    fn everything_outside_the_grammar_is_refused() {
        for command in [
            // Unquoted expansion, substitution, backticks, ANSI quoting.
            r#"for d in /a; do echo x; (cd $d && claude -p "q"); done"#,
            r#"Q=$(cat f); (cd /a && claude -p "$Q")"#,
            r#"(cd /a && claude -p "`date`")"#,
            r#"(cd /a && claude -p $'q')"#,
            // An unknown variable, an environment-steering assignment.
            r#"(cd /a && claude -p "$UNSET")"#,
            r#"HOME=/x; (cd /a && claude -p q)"#,
            r#"CLAUDE_CONFIG_DIR=/x; (cd /a && claude -p q)"#,
            // A prefix assignment, a top-level launch, a relative directory.
            r#"Q=x claude -p q"#,
            r#"claude -p q"#,
            r#"(cd a && claude -p q)"#,
            r#"(cd /a/../b && claude -p q)"#,
            // Resume, continue, fork, background, no saved session, help,
            // a subcommand, an unknown option, no print, two prompts, a
            // prompt a many-valued option would take.
            r#"(cd /a && claude -p q --resume x)"#,
            r#"(cd /a && claude -p q -c)"#,
            r#"(cd /a && claude -p q --fork-session)"#,
            r#"(cd /a && claude -p q --bg)"#,
            r#"(cd /a && claude -p q --no-session-persistence)"#,
            r#"(cd /a && claude -p q --help)"#,
            r#"(cd /a && claude -p doctor)"#,
            r#"(cd /a && claude -p q --agent x)"#,
            r#"(cd /a && claude -p q --frobnicate)"#,
            r#"(cd /a && claude q)"#,
            r#"(cd /a && claude -p q r)"#,
            r#"(cd /a && claude -p --allowedTools Bash q)"#,
            r#"(cd /a && claude -p q --output-format yaml)"#,
            r#"(cd /a && claude -p q --input-format stream-json)"#,
            // Other redirections and pipes, background, lists.
            r#"(cd /a && claude -p q 2>/dev/null)"#,
            r#"(cd /a && claude -p q > out)"#,
            r#"(cd /a && claude -p q | head -4)"#,
            r#"(cd /a && claude -p q | tail -0)"#,
            r#"(cd /a && claude -p q) &"#,
            r#"(cd /a && claude -p q) || true"#,
            r#"(cd /a; claude -p q)"#,
            // Two launches with no marker between them.
            r#"for d in /a /b; do (cd "$d" && claude -p q); done"#,
            // Nested loops, globs, tilde, comments, newlines, echo options.
            r#"for a in /x; do for b in /y; do echo z; done; done"#,
            r#"for d in /a/*; do echo x; (cd "$d" && claude -p q); done"#,
            r#"(cd ~/a && claude -p q)"#,
            "echo x # c\n(cd /a && claude -p q)",
            r#"echo -e "x"; (cd /a && claude -p q)"#,
            r#"echo "a\nb"; (cd /a && claude -p q)"#,
            // No launch at all.
            r#"echo hello"#,
        ] {
            assert!(launches(command).is_none(), "{command}");
        }
    }

    /// Streaming JSON and verbose launches are read as launches of their own
    /// format: recognized, so the call keeps its place, but only a plain
    /// answer or one JSON result is ever matched (see `parent`).
    #[test]
    fn streaming_and_verbose_launches_are_recognized_with_their_format() {
        for (options, format, verbose) in [
            ("", Format::Text, false),
            ("--output-format text", Format::Text, false),
            ("--output-format=json", Format::Json, false),
            (
                "--verbose --output-format stream-json",
                Format::StreamJson,
                true,
            ),
            ("--verbose", Format::Text, true),
            ("--output-format json --verbose", Format::Json, true),
        ] {
            let command = format!("(cd /a && claude -p q {options} 2>&1)");
            let launch = &launches(&command).expect(&command)[0];
            assert_eq!(
                (launch.format, launch.verbose),
                (format, verbose),
                "{command}"
            );
        }
    }

    /// The map's neutral options, aliases and `=` spellings read as they do
    /// in the Codex launch form, whatever their order.
    #[test]
    fn neutral_options_of_the_shared_map_are_read() {
        let command = r#"(cd /a && claude --name "Task" --no-chrome --tools Read,Grep --allowed-tools Bash -p --mcp-config '{"mcpServers":{}}' --session-id=0c000000-0000-4000-8000-0000000000c1 --effort=high -n=x q)"#;
        assert!(launches(command).is_none(), "a short option with `=`");
        let command = r#"(cd /a && claude --name "Task" --no-chrome --tools Read,Grep --allowed-tools Bash -p --mcp-config '{"mcpServers":{}}' --session-id=0c000000-0000-4000-8000-0000000000c1 --effort=high --max-turns 2 --safe-mode q)"#;
        let launch = &launches(command).unwrap()[0];
        assert_eq!(launch.prompt, "q");
        assert_eq!(
            launch.session_id.as_deref(),
            Some("0c000000-0000-4000-8000-0000000000c1")
        );
    }

    /// Both Claude readers take every option from the one shared map: for
    /// each spelling of each described option, with a well-formed value
    /// where it takes one, the Codex launch form and this reader accept or
    /// refuse alike, and accept exactly when the map reads a fresh print
    /// session.
    #[test]
    fn both_readers_read_options_from_the_one_shared_map() {
        use super::super::super::command;
        const CHILD: &str = "0c000000-0000-4000-8000-0000000000c1";
        let value = |opt: &options::Opt| match (opt.meaning, opt.arity) {
            (options::Meaning::SessionId, _) => Some(CHILD),
            (options::Meaning::OutputFormat | options::Meaning::InputFormat, _) => Some("text"),
            (options::Meaning::MaxTurns, _) => Some("3"),
            (options::Meaning::McpConfig, _) => Some(r#"{"mcpServers":{}}"#),
            (options::Meaning::Budget, _) => Some("1.5"),
            (_, options::Arity::Switch) => None,
            (_, options::Arity::Optional) => Some(CHILD),
            _ => Some("v"),
        };
        let mut compared = 0;
        for opt in options::OPTIONS {
            for name in opt.names {
                let given = match value(opt) {
                    Some(value) if name.starts_with("--") => vec![format!("{name}={value}")],
                    Some(value) => vec![name.to_string(), value.to_owned()],
                    None => vec![name.to_string()],
                };
                // `-p`, the session and the prompt, with this option first.
                let mut args = given.clone();
                if opt.meaning != options::Meaning::Print {
                    args.push("-p".into());
                }
                if opt.meaning != options::Meaning::SessionId {
                    args.extend(["--session-id".into(), CHILD.into()]);
                }
                args.push("prompt".into());
                let read = options::read(&args);
                let create = read.as_ref().map(|read| read.action) == Some(Action::Create);
                let line = args
                    .iter()
                    .map(|arg| format!("'{arg}'"))
                    .collect::<Vec<_>>()
                    .join(" ");
                let codex = command::launch(&format!("claude {line}"), &[]).is_some();
                let bash = launches(&format!("(cd /a && claude {line})")).is_some();
                assert_eq!((codex, bash), (create, create), "{line}: {read:?}");
                compared += 1;
            }
        }
        assert!(compared >= options::OPTIONS.len());
        // Values and prerequisites the CLI refuses before starting a
        // session are refused alike; their valid forms are read alike.
        for (extra, create) in [
            ("--output-format stream-json", false),
            ("--output-format=stream-json", false),
            ("--output-format stream-json --verbose", true),
            ("--verbose --output-format=stream-json", true),
            ("--max-budget-usd nope", false),
            ("--max-budget-usd 0", false),
            ("--max-budget-usd 5", true),
            ("--max-budget-usd=2.50", true),
            ("--mcp-config '{}'", false),
            (r#"--mcp-config '{"mcpServers":{}}'"#, true),
        ] {
            let line = format!("-p --session-id {CHILD} {extra} --safe-mode prompt");
            let codex = command::launch(&format!("claude {line}"), &[]).is_some();
            let bash = launches(&format!("(cd /a && claude {line})")).is_some();
            assert_eq!((codex, bash), (create, create), "{line}");
        }
    }

    fn two() -> Script {
        parse(LOOP, &[]).unwrap()
    }

    #[test]
    fn the_output_splits_into_each_complete_answer() {
        let script = two();
        let printed =
            "== started in /work/alpha\nFirst answer.\n== started in /work/beta\nSecond\nanswer.\n";
        assert_eq!(
            partition(&script, printed),
            Some(vec!["First answer.", "Second\nanswer."])
        );
        // Only the very last newline may be missing.
        assert_eq!(
            partition(&script, printed.strip_suffix('\n').unwrap()),
            Some(vec!["First answer.", "Second\nanswer."])
        );
    }

    #[test]
    fn unexplained_missing_or_colliding_output_is_refused() {
        let script = two();
        for printed in [
            // An empty launch output, a missing marker, extra text first.
            "== started in /work/alpha\n== started in /work/beta\nB\n",
            "== started in /work/alpha\nA\nB\n",
            "noise\n== started in /work/alpha\nA\n== started in /work/beta\nB\n",
            // A marker line inside an answer.
            "== started in /work/alpha\nA\n== started in /work/alpha\n== started in /work/beta\nB\n",
            // More lines than `tail -4` prints.
            "== started in /work/alpha\n1\n2\n3\n4\n5\n== started in /work/beta\nB\n",
            "",
        ] {
            assert_eq!(partition(&script, printed), None, "{printed:?}");
        }
    }
}
