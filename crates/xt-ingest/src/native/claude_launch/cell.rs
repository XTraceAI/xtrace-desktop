//! A restricted reader for the JavaScript cells a Codex agent logs for its
//! `exec` tool: the reviewed one-operation reader, extended to a bounded
//! sequence of operations.
//!
//! Nothing here evaluates code. A cell is string constants and one or more
//! awaited `tools.<name>(...)` operations, in any order, each of whose whole
//! result is emitted once by `text` immediately, so the cell's emitted
//! results are its operations' in order, one each:
//!
//! ```text
//! const p = "...";                                   // any number
//! text(await tools.exec_command({cmd: "...", ...}))  // or
//! const r = await tools.exec_command({...}); text(r)
//! ```
//!
//! `exec_command` and `write_stdin` take one object of arguments. An argument
//! is a string, number or boolean literal, a constant, or
//! `<constant>.replace(/'/g, "<string>")`, and strings may be joined with `+`.
//!
//! Any other tool is an opaque operation: it keeps its place, and so its
//! result's, but nothing of it is kept. Its arguments must still be plain
//! data — the values above, `null`, JSON numbers (`-1`, `1.5`, `1e-3`),
//! template literals without substitutions, and objects and arrays of these,
//! at most [`MAX_DEPTH`] levels counting the argument itself as the first —
//! and are read only to find where the call ends. `exec_command` and
//! `write_stdin` numbers stay unsigned integers.
//!
//! Everything else is refused: other statements, calls, member access,
//! arithmetic, substitutions, comments and other regular expressions.

use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(String),
    Punct(&'static str),
    Regex(String),
    /// A template literal without substitutions; its text is not kept.
    Template,
    /// A JSON number that is not an unsigned integer — signed, with a
    /// fraction or an exponent; only opaque data takes it, and it is not kept.
    OpaqueNum,
}

const PUNCTS: [&str; 12] = ["{", "}", "(", ")", "[", "]", ",", ";", ":", "+", "=", "."];

fn tokenize(source: &str) -> Result<Vec<Tok>, ()> {
    let chars: Vec<char> = source.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if matches!(c, ' ' | '\t' | '\n' | '\r') {
            index += 1;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let start = index;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                index += 1;
            }
            tokens.push(Tok::Ident(chars[start..index].iter().collect()));
            continue;
        }
        if c.is_ascii_digit() || c == '-' {
            let (token, next) = number(&chars, index)?;
            tokens.push(token);
            index = next;
            continue;
        }
        if c == '"' || c == '\'' {
            let (value, next) = string(&chars, index)?;
            tokens.push(Tok::Str(value));
            index = next;
            continue;
        }
        if c == '`' {
            index = template(&chars, index)?;
            tokens.push(Tok::Template);
            continue;
        }
        if c == '/' {
            // Only `/'/g`, and only where an argument starts.
            if !matches!(tokens.last(), Some(Tok::Punct("("))) {
                return Err(());
            }
            let regex: String = chars[index..(index + 4).min(chars.len())].iter().collect();
            if regex != "/'/g"
                || chars
                    .get(index + 4)
                    .is_some_and(char::is_ascii_alphanumeric)
            {
                return Err(());
            }
            tokens.push(Tok::Regex(regex));
            index += 4;
            continue;
        }
        let punct = PUNCTS.iter().find(|punct| punct.starts_with(c)).ok_or(())?;
        tokens.push(Tok::Punct(punct));
        index += 1;
    }
    Ok(tokens)
}

/// A quoted string literal with the escapes JSON and JavaScript share.
fn string(chars: &[char], start: usize) -> Result<(String, usize), ()> {
    let quote = chars[start];
    let mut value = String::new();
    let mut index = start + 1;
    loop {
        let &c = chars.get(index).ok_or(())?;
        match c {
            _ if c == quote => return Ok((value, index + 1)),
            '\n' | '\r' => return Err(()),
            '\\' => {
                let escape = *chars.get(index + 1).ok_or(())?;
                index += 2;
                match escape {
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    'b' => value.push('\u{8}'),
                    'f' => value.push('\u{c}'),
                    'v' => value.push('\u{b}'),
                    '"' | '\'' | '\\' | '/' => value.push(escape),
                    '0' if !chars.get(index).is_some_and(char::is_ascii_digit) => value.push('\0'),
                    'u' => {
                        let code = hex(chars, index, 4)?;
                        index += 4;
                        let unit = if (0xD800..0xDC00).contains(&code) {
                            // A surrogate pair spells one character.
                            if chars.get(index..index + 2) != Some(&['\\', 'u']) {
                                return Err(());
                            }
                            let low = hex(chars, index + 2, 4)?;
                            if !(0xDC00..0xE000).contains(&low) {
                                return Err(());
                            }
                            index += 6;
                            0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00)
                        } else {
                            code
                        };
                        value.push(char::from_u32(unit).ok_or(())?);
                    }
                    'x' => {
                        let unit = hex(chars, index, 2)?;
                        value.push(char::from_u32(unit).ok_or(())?);
                        index += 2;
                    }
                    _ => return Err(()),
                }
            }
            _ => {
                value.push(c);
                index += 1;
            }
        }
    }
}

/// A number in JSON's spelling. An unsigned integer is a `Num`, its digits
/// as written; anything else must be canonical JSON and is an `OpaqueNum`.
fn number(chars: &[char], start: usize) -> Result<(Tok, usize), ()> {
    let digits = |mut index: usize| -> Result<usize, ()> {
        let from = index;
        while chars.get(index).is_some_and(char::is_ascii_digit) {
            index += 1;
        }
        if index == from { Err(()) } else { Ok(index) }
    };
    let signed = chars[start] == '-';
    let integer = start + usize::from(signed);
    let mut index = digits(integer)?;
    let unsigned_integer = index;
    if chars.get(index) == Some(&'.') {
        index = digits(index + 1)?;
    }
    if matches!(chars.get(index), Some('e' | 'E')) {
        index += 1;
        if matches!(chars.get(index), Some('+' | '-')) {
            index += 1;
        }
        index = digits(index)?;
    }
    // Nothing may run on: `1n`, `0x1`, `1.5.2`, `1_000`.
    if chars
        .get(index)
        .is_some_and(|&c| c.is_ascii_alphanumeric() || c == '_' || c == '.')
    {
        return Err(());
    }
    if !signed && index == unsigned_integer {
        return Ok((Tok::Num(chars[start..index].iter().collect()), index));
    }
    if chars[integer] == '0' && unsigned_integer - integer > 1 {
        return Err(());
    }
    Ok((Tok::OpaqueNum, index))
}

/// The end of a template literal, refusing one with a substitution.
fn template(chars: &[char], start: usize) -> Result<usize, ()> {
    let mut index = start + 1;
    loop {
        match *chars.get(index).ok_or(())? {
            '`' => return Ok(index + 1),
            '\\' => {
                chars.get(index + 1).ok_or(())?;
                index += 2;
            }
            '$' if chars.get(index + 1) == Some(&'{') => return Err(()),
            _ => index += 1,
        }
    }
}

fn hex(chars: &[char], at: usize, len: usize) -> Result<u32, ()> {
    let digits = chars.get(at..at + len).ok_or(())?;
    if !digits.iter().all(char::is_ascii_hexdigit) {
        return Err(());
    }
    u32::from_str_radix(&digits.iter().collect::<String>(), 16).map_err(|_| ())
}

/// A value an argument object may hold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Value {
    Str(String),
    Num(String),
    Bool(bool),
}

/// The tool an operation calls.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Tool {
    ExecCommand,
    WriteStdin,
    /// Any other tool: its place only, never its arguments.
    Opaque,
}

/// One operation of a cell; its result is the cell's emitted item at its
/// position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Cell {
    pub tool: Tool,
    pub args: BTreeMap<String, Value>,
}

impl Cell {
    pub fn str(&self, key: &str) -> Option<&str> {
        match self.args.get(key)? {
            Value::Str(value) => Some(value),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn num(&self, key: &str) -> Option<&str> {
        match self.args.get(key)? {
            Value::Num(value) => Some(value),
            _ => None,
        }
    }
}

/// Names that are never a binding.
const RESERVED: [&str; 9] = [
    "const",
    "await",
    "text",
    "tools",
    "true",
    "false",
    "null",
    "replace",
    "undefined",
];

struct Parser {
    tokens: Vec<Tok>,
    at: usize,
    strings: BTreeMap<String, String>,
    /// Names already bound to an operation's result.
    results: Vec<String>,
}

/// The most operations one cell may hold.
pub(super) const MAX_OPERATIONS: usize = 32;

/// The most levels an opaque operation's argument may nest, the argument
/// itself being the first: 16 nested arrays, or 15 around a scalar.
pub(super) const MAX_DEPTH: usize = 16;

/// The one operation of a cell, or `Err` for anything outside the grammar or
/// a cell of several operations.
#[cfg(test)]
pub(super) fn parse_cell(source: &str) -> Result<Cell, ()> {
    match <[Cell; 1]>::try_from(parse_operations(source)?) {
        Ok([cell]) => Ok(cell),
        Err(_) => Err(()),
    }
}

/// Every operation of a cell, in order, or `Err` for anything outside the
/// grammar. Each operation's result is emitted once, right after it, so the
/// cell's `n`-th emitted result is its `n`-th operation's.
pub(super) fn parse_operations(source: &str) -> Result<Vec<Cell>, ()> {
    let mut parser = Parser {
        tokens: tokenize(source)?,
        at: 0,
        strings: BTreeMap::new(),
        results: Vec::new(),
    };
    let mut cells = Vec::new();
    while parser.at < parser.tokens.len() {
        if parser.is_keyword("text") {
            parser.at += 1;
            parser.expect("(")?;
            parser.keyword("await")?;
            cells.push(parser.tool_call()?);
            parser.expect(")")?;
        } else {
            parser.keyword("const")?;
            let name = parser.binding()?;
            parser.expect("=")?;
            if parser.is_keyword("await") {
                parser.at += 1;
                cells.push(parser.tool_call()?);
                parser.eat(";");
                parser.keyword("text")?;
                parser.expect("(")?;
                if parser.ident()? != name {
                    return Err(());
                }
                parser.expect(")")?;
                // The result's name is spent: it is never a constant.
                parser.results.push(name);
            } else {
                let value = parser.string()?;
                parser.strings.insert(name, value);
            }
        }
        parser.eat(";");
        if cells.len() > MAX_OPERATIONS {
            return Err(());
        }
    }
    if cells.is_empty() {
        return Err(());
    }
    Ok(cells)
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.at)
    }

    fn eat(&mut self, punct: &str) -> bool {
        if matches!(self.peek(), Some(Tok::Punct(found)) if *found == punct) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, punct: &str) -> Result<(), ()> {
        if self.eat(punct) { Ok(()) } else { Err(()) }
    }

    fn ident(&mut self) -> Result<String, ()> {
        match self.peek() {
            Some(Tok::Ident(name)) => {
                let name = name.clone();
                self.at += 1;
                Ok(name)
            }
            _ => Err(()),
        }
    }

    fn keyword(&mut self, word: &str) -> Result<(), ()> {
        if self.ident()? == word {
            Ok(())
        } else {
            Err(())
        }
    }

    fn is_keyword(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(name)) if name == word)
    }

    /// A new, unreserved name.
    fn binding(&mut self) -> Result<String, ()> {
        let name = self.ident()?;
        if RESERVED.contains(&name.as_str())
            || self.strings.contains_key(&name)
            || self.results.contains(&name)
        {
            return Err(());
        }
        Ok(name)
    }

    fn string(&mut self) -> Result<String, ()> {
        match self.peek() {
            Some(Tok::Str(value)) => {
                let value = value.clone();
                self.at += 1;
                Ok(value)
            }
            _ => Err(()),
        }
    }

    /// `tools.<exec_command|write_stdin>({<key>: <expression>, ...})`, or
    /// `tools.<other>(<data>, ...)`.
    fn tool_call(&mut self) -> Result<Cell, ()> {
        self.keyword("tools")?;
        self.expect(".")?;
        let tool = match self.ident()?.as_str() {
            "exec_command" => Tool::ExecCommand,
            "write_stdin" => Tool::WriteStdin,
            _ => Tool::Opaque,
        };
        self.expect("(")?;
        if tool == Tool::Opaque {
            while !self.eat(")") {
                self.data(0)?;
                if !self.eat(",") {
                    self.expect(")")?;
                    break;
                }
            }
            return Ok(Cell {
                tool,
                args: BTreeMap::new(),
            });
        }
        self.expect("{")?;
        let mut args = BTreeMap::new();
        while !self.eat("}") {
            let key = match self.peek().cloned() {
                Some(Tok::Ident(key) | Tok::Str(key)) => {
                    self.at += 1;
                    key
                }
                _ => return Err(()),
            };
            self.expect(":")?;
            let value = self.expression()?;
            if args.insert(key, value).is_some() {
                return Err(());
            }
            if !self.eat(",") {
                self.expect("}")?;
                break;
            }
        }
        self.expect(")")?;
        Ok(Cell { tool, args })
    }

    /// An opaque operation's argument, read and dropped: an expression,
    /// `null`, a JSON number, a template literal, or an object or array of
    /// these.
    fn data(&mut self, depth: usize) -> Result<(), ()> {
        if depth >= MAX_DEPTH {
            return Err(());
        }
        let close = match self.peek() {
            Some(Tok::Template | Tok::OpaqueNum) => {
                self.at += 1;
                return Ok(());
            }
            // JSON's integers: no leading zero.
            Some(Tok::Num(digits)) if digits.len() > 1 && digits.starts_with('0') => {
                return Err(());
            }
            Some(Tok::Ident(name)) if name == "null" => {
                self.at += 1;
                return Ok(());
            }
            Some(Tok::Punct("{")) => "}",
            Some(Tok::Punct("[")) => "]",
            _ => return self.expression().map(drop),
        };
        self.at += 1;
        while !self.eat(close) {
            if close == "}" {
                match self.peek() {
                    Some(Tok::Ident(_) | Tok::Str(_)) => self.at += 1,
                    _ => return Err(()),
                }
                self.expect(":")?;
            }
            self.data(depth + 1)?;
            if !self.eat(",") {
                self.expect(close)?;
                break;
            }
        }
        Ok(())
    }

    /// `TERM ("+" TERM)*`; only strings concatenate.
    fn expression(&mut self) -> Result<Value, ()> {
        let first = self.term()?;
        if !matches!(self.peek(), Some(Tok::Punct("+"))) {
            return Ok(first);
        }
        let Value::Str(mut text) = first else {
            return Err(());
        };
        while self.eat("+") {
            match self.term()? {
                Value::Str(more) => text.push_str(&more),
                Value::Num(_) | Value::Bool(_) => return Err(()),
            }
        }
        Ok(Value::Str(text))
    }

    /// A string, number or boolean literal, a constant, or
    /// `<constant>.replace(/'/g, "<string>")`.
    fn term(&mut self) -> Result<Value, ()> {
        match self.peek().cloned() {
            Some(Tok::Str(value)) => {
                self.at += 1;
                Ok(Value::Str(value))
            }
            Some(Tok::Num(value)) => {
                self.at += 1;
                Ok(Value::Num(value))
            }
            Some(Tok::Ident(name)) if name == "true" || name == "false" => {
                self.at += 1;
                Ok(Value::Bool(name == "true"))
            }
            Some(Tok::Ident(name)) => {
                self.at += 1;
                let bound = self.strings.get(&name).cloned().ok_or(())?;
                if !self.eat(".") {
                    return Ok(Value::Str(bound));
                }
                self.keyword("replace")?;
                self.expect("(")?;
                if self.peek() != Some(&Tok::Regex("/'/g".into())) {
                    return Err(());
                }
                self.at += 1;
                self.expect(",")?;
                let with = self.string()?;
                self.expect(")")?;
                Ok(Value::Str(bound.replace('\'', &with)))
            }
            _ => Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_inline_and_bound_launch_forms() {
        let cell = parse_cell(
            "const p=\"it's \\\"quoted\\\"\\n\";\ntext(await tools.exec_command({cmd:\"claude -p '\"+p.replace(/'/g,\"'\\\\''\")+\"'\",\"workdir\":\"/w\",\"yield_time_ms\":1000,tty:false}));",
        )
        .unwrap();
        assert_eq!(cell.tool, Tool::ExecCommand);
        assert_eq!(
            cell.str("cmd").unwrap(),
            "claude -p 'it'\\''s \"quoted\"\n'"
        );
        assert_eq!(cell.num("yield_time_ms"), Some("1000"));
        assert_eq!(cell.args.get("tty"), Some(&Value::Bool(false)));
        let cell =
            parse_cell("const r = await tools.write_stdin({session_id: 7, chars: \"\"});\ntext(r)")
                .unwrap();
        assert_eq!(cell.tool, Tool::WriteStdin);
        assert_eq!(cell.num("session_id"), Some("7"));
        assert_eq!(cell.str("chars"), Some(""));
        assert_eq!(
            parse_cell("text(await tools.exec_command({cmd:\"\\u00e9\\ud83d\\ude00\"}))")
                .unwrap()
                .str("cmd"),
            Some("é😀")
        );
    }

    #[test]
    fn refuses_everything_outside_the_grammar() {
        for source in [
            "",
            "const p=`x ${y}`;",
            "eval(\"x\")",
            "text(await tools.exec_command({cmd:\"a\"+(1+2)}))",
            "text(await tools.exec_command({cmd:p}))",
            "text(await tools.exec_command({cmd:\"a\"+1}))",
            "const p=\"a\";text(await tools.exec_command({cmd:p.replace(/a/g,\"b\")}))",
            "const p=\"a\";text(await tools.exec_command({cmd:p.replace(/'/gi,\"b\")}))",
            "const p=\"a\";text(await tools.exec_command({cmd:p.slice(1)}))",
            "const p=\"/a\";const q=p;text(await tools.exec_command({cmd:q}))",
            "text(await tools.exec_command({cmd:`a`}))",
            "text(await tools.exec_command({cmd:\"a\"},{x:1}))",
            "text(await tools.exec_command(\"a\"))",
            "text(await tools.exec_command({cmd:\"a\"}))// c",
            "text(await tools.exec_command({cmd:\"a\"}))/* c */",
            "const p=\"a\";const p=\"b\";text(await tools.exec_command({cmd:p}))",
            "const text=\"a\";text(await tools.exec_command({cmd:\"a\"}))",
            "text(await tools.exec_command({cmd:\"a\",cmd:\"b\"}))",
            "text(await tools.exec_command({cmd:\"\\q\"}))",
            "text(await tools.exec_command({cmd:\"\\u12\"}))",
            "const r=await tools.exec_command({cmd:\"a\"});text(r.output)",
            "const r=await tools.exec_command({cmd:\"a\"});text(s)",
            "const r=await tools.exec_command({cmd:\"a\"})",
            "tools.exec_command({cmd:\"a\"})",
            "text(tools.exec_command({cmd:\"a\"}))",
            "text(await tools.exec_command({...o}))",
            "text(await tools.exec_command({cmd:[\"a\"]}))",
            "text(await tools.exec_command({cmd:null}))",
            "if(x)text(y)",
            "const null=\"a\";text(await tools.exec_command({cmd:null}))",
        ] {
            assert!(parse_cell(source).is_err(), "{source}");
            assert!(parse_operations(source).is_err(), "{source}");
        }
    }

    #[test]
    fn reads_a_sequence_of_operations_each_emitted_once_in_order() {
        let cells = parse_operations(
            "const r = await tools.exec_command({cmd: \"test -s /w/o.json\"});\ntext(r);\nconst s = await tools.write_stdin({session_id: 56751, chars: \"\", yield_time_ms: 30000});\ntext(s)",
        )
        .unwrap();
        assert_eq!(cells.len(), 2);
        assert_eq!(cells[0].tool, Tool::ExecCommand);
        assert_eq!(cells[1].tool, Tool::WriteStdin);
        assert_eq!(cells[1].num("session_id"), Some("56751"));
        let cells = parse_operations(
            "text(await tools.write_stdin({session_id: 7, chars: \"\"}));\nconst p = \"x\";\ntext(await tools.exec_command({cmd: p}))",
        )
        .unwrap();
        assert_eq!(
            cells.iter().map(|cell| cell.tool).collect::<Vec<_>>(),
            [Tool::WriteStdin, Tool::ExecCommand]
        );
        // A single-operation reader refuses a sequence.
        assert!(
            parse_cell("text(await tools.exec_command({cmd:\"a\"}));text(await tools.exec_command({cmd:\"b\"}))")
                .is_err()
        );
        let many = "text(await tools.exec_command({cmd:\"a\"}));".repeat(MAX_OPERATIONS + 1);
        assert!(parse_operations(&many).is_err());
        for source in [
            // A result emitted late, twice, never, or rebound.
            "const r = await tools.exec_command({cmd:\"a\"}); const s = await tools.exec_command({cmd:\"b\"}); text(r); text(s)",
            "const r = await tools.exec_command({cmd:\"a\"}); text(r); text(r)",
            "const r = await tools.exec_command({cmd:\"a\"}); text(r); const r = await tools.exec_command({cmd:\"b\"}); text(r)",
            "const r = await tools.exec_command({cmd:\"a\"}); text(r); text(await tools.exec_command({cmd:r}))",
            "text(await tools.exec_command({cmd:\"a\"})) text(await tools.exec_command({cmd:\"b\"})) x",
            "await tools.exec_command({cmd:\"a\"}); text(await tools.exec_command({cmd:\"b\"}))",
            "const p = \"a\";",
            "",
        ] {
            assert!(parse_operations(source).is_err(), "{source}");
        }
    }

    /// Other tools keep their places, and their results', whatever their
    /// plain-data arguments say; nothing of them is kept.
    #[test]
    fn other_tools_are_opaque_operations_in_their_places() {
        let kinds = |source: &str| {
            parse_operations(source)
                .unwrap_or_else(|()| panic!("{source}"))
                .iter()
                .map(|cell| cell.tool)
                .collect::<Vec<_>>()
        };
        let launch = "text(await tools.exec_command({cmd:\"claude -p x\"}))";
        let patch = "text(await tools.apply_patch(\"*** Begin Patch\\n+claude -p --session-id 0c000000-0000-4000-8000-0000000000c1 x\\n*** End Patch\"))";
        assert_eq!(
            kinds(&format!(
                "{patch};\n{launch};\nconst r = await tools.update_plan({{plan: [{{step: \"a\", status: \"done\"}}], n: 56751, ok: true, none: null}});\ntext(r)"
            )),
            [Tool::Opaque, Tool::ExecCommand, Tool::Opaque]
        );
        assert_eq!(
            kinds(&format!("{launch};{patch};{launch}")),
            [Tool::ExecCommand, Tool::Opaque, Tool::ExecCommand]
        );
        for source in [
            "text(await tools.apply_patch(`*** Begin Patch\n+a \\` b \\${c} $d {e}\n*** End Patch`))",
            "const p = \"x\";\ntext(await tools.view_image({path: p + \"/a\", \"detail\": \"low\"}))",
            "const p = \"x\";\ntext(await tools.note(p.replace(/'/g, \"y\"), [], {}, [[1, [true]]],))",
            "text(await tools.list_mcp_resources())",
        ] {
            let cells = parse_operations(source).unwrap_or_else(|()| panic!("{source}"));
            assert_eq!(cells.len(), 1, "{source}");
            assert_eq!(cells[0].tool, Tool::Opaque, "{source}");
            assert!(cells[0].args.is_empty(), "{source}");
        }
        let deep = |depth: usize| {
            format!(
                "text(await tools.note({}{}))",
                "[".repeat(depth),
                "]".repeat(depth)
            )
        };
        assert!(parse_operations(&deep(MAX_DEPTH)).is_ok());
        for source in [
            // Something run, read from a result, or computed.
            "text(await tools.apply_patch(f()))",
            "text(await tools.apply_patch(tools.exec_command({cmd:\"a\"})))",
            "text(await tools.apply_patch(await tools.exec_command({cmd:\"a\"})))",
            "const r = await tools.exec_command({cmd:\"a\"}); text(r); text(await tools.apply_patch(r))",
            "text(await tools.apply_patch(`a ${b} c`))",
            "text(await tools.apply_patch(`a ${\"b\"} c`))",
            "text(await tools.apply_patch(`a` + \"b\"))",
            "text(await tools.apply_patch(x))",
            "text(await tools.apply_patch({x}))",
            "text(await tools.apply_patch({...o}))",
            "text(await tools.apply_patch({[k]: 1}))",
            "text(await tools.apply_patch([...a]))",
            "text(await tools.apply_patch([1,,2]))",
            "text(await tools.apply_patch({a: 1 + 2}))",
            "text(await tools.apply_patch(\"a\".length))",
            "text(await tools.apply_patch(`a`)",
            "text(await tools.apply_patch(`a))",
            "text(await tools.apply_patch(/'/g))",
            "text(await tools.a.b(\"x\"))",
            "text(await tools[\"apply_patch\"](\"x\"))",
            // A result emitted late, never, twice, or in part.
            "const r = await tools.apply_patch(\"x\"); text(await tools.exec_command({cmd:\"a\"})); text(r)",
            "await tools.apply_patch(\"x\"); text(await tools.exec_command({cmd:\"a\"}))",
            "const r = await tools.apply_patch(\"x\"); text(r); text(r)",
            "const r = await tools.apply_patch(\"x\"); text(r.output)",
            "text(await tools.apply_patch(\"x\")) ? text(\"a\") : 0",
            "if (ok) text(await tools.apply_patch(\"x\"))",
            "for (const x of [1]) text(await tools.apply_patch(\"x\"))",
        ] {
            assert!(parse_operations(source).is_err(), "{source}");
        }
        assert!(parse_operations(&deep(MAX_DEPTH + 1)).is_err());
        let populated = |depth: usize| {
            format!(
                "text(await tools.note({}1.5{}))",
                "[".repeat(depth),
                "]".repeat(depth)
            )
        };
        assert!(parse_operations(&populated(MAX_DEPTH - 1)).is_ok());
        assert!(parse_operations(&populated(MAX_DEPTH)).is_err());
    }

    /// Other tools' arguments may hold any JSON number, read and dropped;
    /// nothing is computed. `exec_command` and `write_stdin` numbers stay
    /// unsigned integers, as written.
    #[test]
    fn other_tools_take_json_numbers_and_known_tools_do_not() {
        for number in [
            "0", "7", "56751", "-1", "-0", "1.5", "-1.5", "0.25", "1e-3", "1E+10", "-2.5e3", "0e0",
        ] {
            for source in [
                format!("text(await tools.note({number}))"),
                format!("text(await tools.note({{n: {number}, m: [{number}, {{k: {number}}}]}}))"),
                format!("text(await tools.note(\"a\", {number}, null))"),
            ] {
                let cells = parse_operations(&source).unwrap_or_else(|()| panic!("{source}"));
                assert_eq!(cells[0].tool, Tool::Opaque, "{source}");
                assert!(cells[0].args.is_empty(), "{source}");
            }
        }
        for number in [
            "1+2",
            "1 + 2",
            "1-2",
            "1 - 2",
            "-",
            "--1",
            "- 1",
            "+1",
            "1.",
            ".5",
            "01",
            "-01",
            "00",
            "01.5",
            "1.5.2",
            "1e",
            "1e+",
            "1e-",
            "1.e3",
            "0x1F",
            "0b1",
            "0o7",
            "1n",
            "1_000",
            "NaN",
            "Infinity",
            "-Infinity",
            "1 2",
            "1e3e3",
            "(1)",
            "-(1)",
        ] {
            for source in [
                format!("text(await tools.note({number}))"),
                format!("text(await tools.note({{n: {number}}}))"),
                format!("text(await tools.note([{number}]))"),
            ] {
                assert!(parse_operations(&source).is_err(), "{source}");
            }
        }
        // Known tools: unchanged unsigned integers only.
        for number in ["-1", "-0", "1.5", "1e3", "1E+3", "0.5"] {
            for source in [
                format!("text(await tools.exec_command({{cmd: \"a\", yield_time_ms: {number}}}))"),
                format!("text(await tools.write_stdin({{session_id: {number}, chars: \"\"}}))"),
                format!("const p = {number}; text(await tools.exec_command({{cmd: \"a\"}}))"),
            ] {
                assert!(parse_operations(&source).is_err(), "{source}");
            }
        }
        let cell = parse_cell(
            "text(await tools.exec_command({cmd: \"a\", yield_time_ms: 007, max_output_tokens: 0}))",
        )
        .unwrap();
        assert_eq!(cell.num("yield_time_ms"), Some("007"));
        assert_eq!(cell.num("max_output_tokens"), Some("0"));
        assert!(parse_operations("text(await tools.note(007))").is_err());
    }
}
