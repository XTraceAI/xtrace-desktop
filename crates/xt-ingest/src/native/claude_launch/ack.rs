//! Following one launch operation from its launch call to its own first
//! process result: the host's acknowledgment that it ran the command.
//!
//! A cell's output is a fixed header — `Script completed` or `Script running
//! with cell ID <n>` — then the results the cell emitted so far, one per
//! operation in order ([`super::cell`]). Results emitted across a running
//! cell's output and the outputs of the `wait`s on that cell are counted in
//! order; the followed operation's result is the one at its position. It
//! acknowledges the launch when it is a process result: an object with a
//! string `output` and either an integer `exit_code` (any code: the command
//! ran, whatever became of it) or a numeric `session_id` (the process is
//! running). A tool error, an approval denial or anything else in its place
//! is not one, and neither is another operation's result or another call's
//! output.
//!
//! The follower stops at that result, even while its cell still runs: it
//! never waits for the rest of the cell, a later poll, the process's exit or
//! anything else, and nothing after the result can undo it. Before it, a
//! missing, repeated or unreadable output of its own calls, a `wait` on its
//! cell it cannot read, or a completed cell whose results are not one per
//! operation ends it with no acknowledgment.
//!
//! A running cell belongs to the exec call whose output announced it: the
//! scan ([`super::scan`]) ends every follower waiting on a cell another call
//! also claims.

use super::rows::{CellHeader, Item, cell_header};
use super::source::{self, Allowance, Reserved};
use serde_json::Value;
use std::collections::HashSet;

/// `wait` options that change nothing but how long it waits and prints.
const WAIT_KEYS: [&str; 3] = ["cell_id", "yield_time_ms", "max_output_tokens"];

/// Why a launch ended with no acknowledgment. Structural only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Broken {
    /// An output of its calls is missing, repeated or unreadable, a cell
    /// holds more results than operations, or the result at its position is
    /// not a process result.
    Output,
    /// The launch ran longer than its bounds before its result.
    TooLong,
    /// What the follower keeps would pass its thread's memory allowance.
    Memory,
}

/// Where the follower is.
#[derive(Debug)]
enum Step {
    /// Waiting for the launch call's own output.
    Cell,
    /// The launch's cell runs as `cell`; waiting for a `wait` on it.
    Waiting { cell: String },
    /// The `wait` call `wait` on the running cell was made; waiting for its
    /// output.
    Wait { wait: String, cell: String },
    /// The acknowledgment was found: nothing more is followed.
    Done,
}

/// The acknowledgment: the call whose output held the launch operation's
/// own result (the launch call, or a `wait` on its cell), and the process
/// handle when the result says the process is running.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Acknowledged {
    pub call: String,
    pub handle: Option<String>,
}

/// One launch operation's follower.
#[derive(Debug)]
pub(super) struct Ack {
    launch: String,
    op: usize,
    ops: usize,
    /// Results the launch's cell emitted so far.
    emitted: usize,
    step: Step,
    /// Every call identifier the follower used: the launch and its waits.
    pub own: HashSet<String>,
    answered: HashSet<String>,
    /// Bytes the identifiers in `own` and `answered` hold.
    ids: usize,
    /// What the follower keeps — its call identifiers and its step —
    /// reserved from its thread's allowance at actual capacity: before it
    /// grows, by at most what the growth may take, then exactly.
    reserved: Reserved,
}

const SLOT: usize = std::mem::size_of::<String>();

/// A cell output: whether it still runs (as which cell), and the results it
/// emitted.
fn cell_output(items: &[String]) -> Result<(Option<String>, &[String]), Broken> {
    let (header, emitted) = items.split_first().ok_or(Broken::Output)?;
    match cell_header(header) {
        CellHeader::Running(cell) => Ok((Some(cell), emitted)),
        CellHeader::Completed => Ok((None, emitted)),
        CellHeader::Other => Err(Broken::Output),
    }
}

impl Ack {
    /// A follower of the launch call `call`'s operation `op` of `ops`.
    pub fn new(call: &str, op: usize, ops: usize, allowance: &Allowance) -> Result<Self, Broken> {
        let mut reserved = allowance
            .reserve(2 * call.len())
            .map_err(|_| Broken::Memory)?;
        let mut own = HashSet::new();
        let ids = source::insert_copy(&mut own, call, &mut reserved).map_err(|_| Broken::Memory)?;
        let mut ack = Self {
            launch: call.to_owned(),
            op,
            ops,
            emitted: 0,
            step: Step::Cell,
            own,
            answered: HashSet::new(),
            ids,
            reserved,
        };
        ack.fit()?;
        Ok(ack)
    }

    /// Bytes the follower keeps now, at actual capacity.
    fn held(&self) -> usize {
        let step = match &self.step {
            Step::Cell | Step::Done => 0,
            Step::Waiting { cell } => cell.capacity(),
            Step::Wait { wait, cell } => wait.capacity() + cell.capacity(),
        };
        source::table_bytes(self.own.capacity(), SLOT)
            + source::table_bytes(self.answered.capacity(), SLOT)
            + self.ids
            + self.launch.capacity()
            + step
    }

    /// The bytes the follower actually keeps, measured from its buffers.
    #[cfg(test)]
    pub fn actual(&self) -> usize {
        let set = |set: &HashSet<String>| {
            source::table_actual(set.capacity(), SLOT)
                + set.iter().map(String::capacity).sum::<usize>()
        };
        let step = match &self.step {
            Step::Cell | Step::Done => 0,
            Step::Waiting { cell } => cell.capacity(),
            Step::Wait { wait, cell } => wait.capacity() + cell.capacity(),
        };
        set(&self.own) + set(&self.answered) + self.launch.capacity() + step
    }

    /// Reserve `extra` bytes more than the follower holds, before a change
    /// that may take them.
    fn room(&mut self, extra: usize) -> Result<(), Broken> {
        let want = self.held().saturating_add(extra);
        if want > self.reserved.bytes() {
            self.reserved.resize(want).map_err(|_| Broken::Memory)?;
        }
        Ok(())
    }

    /// Hold exactly what the follower keeps, after a change.
    fn fit(&mut self) -> Result<(), Broken> {
        let held = self.held();
        self.reserved.resize(held).map_err(|_| Broken::Memory)
    }

    /// The running cell the follower waits on, when it waits on one.
    pub fn cell(&self) -> Option<&str> {
        match &self.step {
            Step::Waiting { cell } | Step::Wait { cell, .. } => Some(cell),
            Step::Cell | Step::Done => None,
        }
    }

    /// Follow one row after the launch call, holding exactly what the
    /// follower keeps afterwards. `Ok(Some(_))` when this row held the launch
    /// operation's own process result.
    pub fn feed(&mut self, item: &Item) -> Result<Option<Acknowledged>, Broken> {
        let step = self.follow(item)?;
        self.fit()?;
        Ok(step)
    }

    fn follow(&mut self, item: &Item) -> Result<Option<Acknowledged>, Broken> {
        if matches!(self.step, Step::Done) {
            return Ok(None);
        }
        if let Item::Output { call_id, .. } = item {
            if self.answered.contains(call_id) {
                return Err(Broken::Output);
            }
            if self.own.contains(call_id) {
                self.ids += source::insert_copy(&mut self.answered, call_id, &mut self.reserved)
                    .map_err(|_| Broken::Memory)?;
            }
        }
        if let Item::Call {
            call_id,
            name,
            custom: false,
            input,
        } = item
            && name == "wait"
            && let Step::Waiting { cell } | Step::Wait { cell, .. } = &self.step
        {
            let args: serde_json::Map<String, Value> =
                serde_json::from_str(input).map_err(|_| Broken::Output)?;
            let named = args.get("cell_id").and_then(Value::as_str);
            if named.is_some() && named != Some(cell.as_str()) {
                // A wait on another cell.
                return Ok(None);
            }
            // One wait at a time, reading only this cell.
            if matches!(self.step, Step::Wait { .. })
                || named.is_none()
                || args.keys().any(|key| !WAIT_KEYS.contains(&key.as_str()))
            {
                return Err(Broken::Output);
            }
            self.ids += source::insert_copy(&mut self.own, call_id, &mut self.reserved)
                .map_err(|_| Broken::Memory)?;
            self.room(call_id.len())?;
            let Step::Waiting { cell } = std::mem::replace(&mut self.step, Step::Done) else {
                unreachable!("checked above");
            };
            self.step = Step::Wait {
                wait: call_id.clone(),
                cell,
            };
            return Ok(None);
        }
        let Item::Output { call_id, items } = item else {
            return Ok(None);
        };
        match &self.step {
            Step::Cell if *call_id == self.launch => {
                let (running, results) = cell_output(items)?;
                self.after_output(call_id, running, results)
            }
            Step::Wait { wait, cell } if call_id == wait => {
                let (running, results) = cell_output(items)?;
                if running.as_ref().is_some_and(|again| again != cell) {
                    return Err(Broken::Output);
                }
                self.after_output(call_id, running, results)
            }
            _ => Ok(None),
        }
    }

    /// An output of the launch's cell, by the call `call`: still running, or
    /// complete, with the results it emitted.
    fn after_output(
        &mut self,
        call: &str,
        running: Option<String>,
        results: &[String],
    ) -> Result<Option<Acknowledged>, Broken> {
        let before = self.emitted;
        self.emitted = before.saturating_add(results.len());
        // More results than operations, or a completed cell that does not
        // hold one per operation: its positions are not known.
        if self.emitted > self.ops || (running.is_none() && self.emitted != self.ops) {
            return Err(Broken::Output);
        }
        if (before..self.emitted).contains(&self.op) {
            let handle = self.settle(&results[self.op - before])?;
            self.step = Step::Done;
            return Ok(Some(Acknowledged {
                call: call.to_owned(),
                handle,
            }));
        }
        // The result is still to come: only while the cell runs.
        let cell = running.ok_or(Broken::Output)?;
        self.room(cell.len())?;
        self.step = Step::Waiting { cell };
        Ok(None)
    }

    /// The launch operation's own result: a process result, with its handle
    /// when the process is running.
    fn settle(&self, text: &str) -> Result<Option<String>, Broken> {
        let result: Value = {
            let _decoding = self
                .reserved
                .allowance()
                .reserve(source::decoded_bound(text.as_bytes()))
                .map_err(|_| Broken::Memory)?;
            serde_json::from_str(text).map_err(|_| Broken::Output)?
        };
        if !result.get("output").is_some_and(Value::is_string) {
            return Err(Broken::Output);
        }
        let handle = match result.get("session_id") {
            None | Some(Value::Null) => None,
            Some(value) => Some(value.as_u64().ok_or(Broken::Output)?.to_string()),
        };
        let exited = match result.get("exit_code") {
            None | Some(Value::Null) => false,
            Some(code) => {
                code.as_i64().ok_or(Broken::Output)?;
                true
            }
        };
        match (exited, handle) {
            (true, None) => Ok(None),
            (false, Some(handle)) => Ok(Some(handle)),
            // Neither, or both: not a process result this version knows.
            _ => Err(Broken::Output),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn allowance() -> Allowance {
        Allowance::new(1 << 20)
    }

    fn wait(id: &str, cell: &str) -> Item {
        Item::Call {
            call_id: id.into(),
            name: "wait".into(),
            custom: false,
            input: json!({"cell_id": cell, "yield_time_ms": 30000}).to_string(),
        }
    }

    fn texts(id: &str, header: String, items: &[&str]) -> Item {
        let mut texts = vec![header];
        texts.extend(items.iter().map(|text| (*text).to_owned()));
        Item::Output {
            call_id: id.into(),
            items: texts,
        }
    }

    fn output(id: &str, items: &[Value]) -> Item {
        let items: Vec<String> = items.iter().map(Value::to_string).collect();
        let items: Vec<&str> = items.iter().map(String::as_str).collect();
        texts(id, "Script completed\nOutput:\n".into(), &items)
    }

    fn running(id: &str, cell: &str, items: &[Value]) -> Item {
        let items: Vec<String> = items.iter().map(Value::to_string).collect();
        let items: Vec<&str> = items.iter().map(String::as_str).collect();
        texts(
            id,
            format!("{}{cell}\n", super::super::rows::RUNNING),
            &items,
        )
    }

    fn live(handle: u64) -> Value {
        json!({"chunk_id": "a", "session_id": handle, "output": ""})
    }

    fn exit(code: i64) -> Value {
        json!({"chunk_id": "d", "exit_code": code, "output": ""})
    }

    fn acknowledged(call: &str, handle: Option<&str>) -> Option<Acknowledged> {
        Some(Acknowledged {
            call: call.into(),
            handle: handle.map(str::to_owned),
        })
    }

    #[test]
    fn any_exit_or_a_running_handle_at_its_position_acknowledges() {
        for (result, handle) in [
            (exit(0), None),
            (exit(1), None),
            (exit(-9), None),
            (live(56751), Some("56751")),
        ] {
            let mut ack = Ack::new("launch", 1, 2, &allowance()).unwrap();
            assert_eq!(
                ack.feed(&output("launch", &[exit(0), result.clone()])),
                Ok(acknowledged("launch", handle)),
                "{result}"
            );
        }
        // Another operation's exit is never the launch's.
        let mut ack = Ack::new("launch", 0, 2, &allowance()).unwrap();
        assert_eq!(
            ack.feed(&texts(
                "launch",
                "Script completed\n".into(),
                &["exec_command failed", &exit(0).to_string()]
            )),
            Err(Broken::Output)
        );
    }

    #[test]
    fn a_tool_error_or_denial_is_not_a_process_result() {
        for text in [
            "exec_command failed: sandbox error".to_owned(),
            json!("approval denied").to_string(),
            json!({"output": "approval denied by the user"}).to_string(),
            json!({"error": "rejected"}).to_string(),
            json!({"exit_code": 0}).to_string(),
            json!({"exit_code": 1.5, "output": ""}).to_string(),
            json!({"exit_code": "0", "output": ""}).to_string(),
            json!({"session_id": "56751", "output": ""}).to_string(),
            json!({"session_id": 1, "exit_code": 0, "output": ""}).to_string(),
        ] {
            let mut ack = Ack::new("launch", 0, 1, &allowance()).unwrap();
            assert_eq!(
                ack.feed(&texts("launch", "Script completed\n".into(), &[&text])),
                Err(Broken::Output),
                "{text}"
            );
        }
    }

    /// The result is taken as soon as the cell emits it, while it still
    /// runs; after it, nothing is followed.
    #[test]
    fn stops_at_its_result_even_while_the_cell_runs() {
        let mut ack = Ack::new("launch", 0, 2, &allowance()).unwrap();
        assert_eq!(
            ack.feed(&running("launch", "9", &[live(7)])),
            Ok(acknowledged("launch", Some("7")))
        );
        assert!(ack.cell().is_none());
        // A repeated output, another wait, anything at all: nothing more.
        for item in [
            running("launch", "9", &[live(7)]),
            wait("w", "9"),
            output("w", &[exit(1)]),
            texts("launch", "garbage".into(), &[]),
        ] {
            assert_eq!(ack.feed(&item), Ok(None), "{item:?}");
        }
    }

    /// A cell that yields before the launch's result: only a wait on that
    /// cell is followed, and the result arrives in its output.
    #[test]
    fn follows_only_its_cells_waits_to_the_result() {
        let mut ack = Ack::new("launch", 1, 3, &allowance()).unwrap();
        assert_eq!(ack.feed(&running("launch", "9", &[exit(0)])), Ok(None));
        assert_eq!(ack.cell(), Some("9"));
        // A wait on another cell and another call's result are not its own.
        assert_eq!(ack.feed(&wait("x", "10")), Ok(None));
        assert_eq!(ack.feed(&output("x", &[live(7)])), Ok(None));
        assert_eq!(ack.feed(&wait("w1", "9")), Ok(None));
        assert_eq!(ack.feed(&running("w1", "9", &[])), Ok(None));
        assert_eq!(ack.feed(&wait("w2", "9")), Ok(None));
        assert_eq!(
            ack.feed(&running("w2", "9", &[exit(3)])),
            Ok(acknowledged("w2", None))
        );
        let own: Vec<&str> = {
            let mut own: Vec<&str> = ack.own.iter().map(String::as_str).collect();
            own.sort_unstable();
            own
        };
        assert_eq!(own, ["launch", "w1", "w2"]);
    }

    #[test]
    fn refuses_missing_extra_repeated_and_unreadable_outputs() {
        let cases: Vec<(usize, usize, Vec<Item>)> = vec![
            // A completed cell without the launch's result, or with more
            // results than operations, or fewer than one each.
            (0, 1, vec![output("launch", &[])]),
            (1, 2, vec![output("launch", &[exit(0)])]),
            (0, 1, vec![output("launch", &[exit(0), exit(0)])]),
            (0, 2, vec![output("launch", &[exit(0)])]),
            (0, 1, vec![running("launch", "3", &[exit(0), exit(0)])]),
            // An unknown header.
            (
                0,
                1,
                vec![texts(
                    "launch",
                    "unexpected header".into(),
                    &[&exit(0).to_string()],
                )],
            ),
            // The launch's output twice.
            (
                1,
                2,
                vec![
                    running("launch", "3", &[exit(0)]),
                    running("launch", "3", &[]),
                ],
            ),
            // A wait's output naming another cell, a wait without a cell, a
            // second wait before the first answered, unknown wait options.
            (
                1,
                2,
                vec![
                    running("launch", "3", &[exit(0)]),
                    wait("w", "3"),
                    running("w", "4", &[]),
                ],
            ),
            (
                1,
                2,
                vec![
                    running("launch", "3", &[exit(0)]),
                    Item::Call {
                        call_id: "w".into(),
                        name: "wait".into(),
                        custom: false,
                        input: "{}".into(),
                    },
                ],
            ),
            (
                1,
                2,
                vec![
                    running("launch", "3", &[exit(0)]),
                    wait("w", "3"),
                    wait("v", "3"),
                ],
            ),
            (
                1,
                2,
                vec![
                    running("launch", "3", &[exit(0)]),
                    Item::Call {
                        call_id: "w".into(),
                        name: "wait".into(),
                        custom: false,
                        input: json!({"cell_id": "3", "terminate": true}).to_string(),
                    },
                ],
            ),
        ];
        for (op, ops, items) in cases {
            let mut ack = Ack::new("launch", op, ops, &allowance()).unwrap();
            let mut result = Ok(None);
            for item in &items {
                result = ack.feed(item);
                if result.is_err() {
                    break;
                }
            }
            assert_eq!(result, Err(Broken::Output), "{op}/{ops} {items:?}");
        }
    }

    /// What a follower keeps — its call identifiers and step — is reserved
    /// at its actual capacity, and given back when it is dropped.
    #[test]
    fn what_a_follower_keeps_is_reserved_at_its_actual_capacity() {
        let roomy = allowance();
        let mut ack = Ack::new("launch", 2, 3, &roomy).unwrap();
        ack.feed(&running("launch", "3", &[exit(0)])).unwrap();
        ack.feed(&wait("w", "3")).unwrap();
        assert!(ack.actual() <= ack.reserved.bytes());
        assert_eq!(roomy.used(), ack.held(), "exactly what it keeps");
        drop(ack);
        assert_eq!(roomy.used(), 0, "everything given back");
        let tight = Allowance::new(8);
        assert_eq!(
            Ack::new("launch", 0, 1, &tight).map(|_| ()),
            Err(Broken::Memory)
        );
    }
}
