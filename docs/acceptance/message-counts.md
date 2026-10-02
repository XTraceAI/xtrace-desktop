# Message and turn counts

`MetricsDb::counts(window, TypingRate::default())` returns local aggregate work
counts from canonical metadata. The default typing rate is 200 characters per
minute; `TypingRate::new` accepts a positive rate and rejects zero. The estimated
field is named `typing_minutes_est`. No record/session identifiers or content are
returned.

The reader selects indexed timestamp candidates, filters exact native instants
into the half-open window, and then sorts each session by instant and UUID.
Timestamp comparison uses the shared precise parser, including fractional digits
beyond nanoseconds and leap seconds. Filtering precedes segmentation; leading
non-human records and trailing unanswered humans do not form turns. Each human
followed by one or more explicit non-human records forms one responding segment.
All structural blocks and tool calls survive usage-response deduplication.

Sessions and explicit assistant-role records are counted structurally. Unknown
human classification makes human messages, turns and typing unknown. A missing
length on a counted human makes typing unknown; missing lengths on non-humans do
not. A missing tool count makes tool calls unknown. Empty windows return measured
zero. The reader consumes stored classification and never reparses content or
replaces unknown classification with a role heuristic.

## Synthetic evidence

The shared fixture loader writes real canonical records through the production
store writer, without injecting classifications for fixture acceptance.

| Fixture | Sessions | Humans | Turns | Assistant records | Tools | Human characters | Typing minutes est. |
| ------- | -------: | -----: | ----: | ----------------: | ----: | ---------------: | ------------------: |
| F1      |        1 |      5 |     5 |                15 |     5 |              135 |               0.675 |
| F8      |        1 |      1 |     1 |                 3 |     1 |                6 |                0.03 |

The segment structure after the shared-view exclusions is:

| Fixture | In-window segment | Humans | Following non-human records | Responding turns |
| ------- | ----------------- | -----: | --------------------------: | ---------------: |
| F1      | 1                 |      1 |                           4 |                1 |
| F1      | 2                 |      1 |                           4 |                1 |
| F1      | 3                 |      1 |                           4 |                1 |
| F1      | 4                 |      1 |                           4 |                1 |
| F1      | 5                 |      1 |                           4 |                1 |
| F8      | Leading records   |      0 |                           9 |                0 |
| F8      | 1                 |      1 |                           2 |                1 |

Appending unanswered humans to F8 increases human count without adding turns.
Changing the rate to 400 halves typing estimates. Full-content and metadata-only
writes produce identical counts. Unicode character counting remains owned by the
writer; tool payloads do not contribute typing characters.

F18's `counts` snapshot separately exercises both native/hook canonical payload
orders, duplicate replay, and both content retention modes. It produces one human,
one turn, two assistant records, two tools and four human characters. This is
canonical store-writer replay coverage, not native-reader or plugin transport
conformance. F18 remains a skeleton for its broader acceptance obligations.

## Checks

```sh
cargo test -p xt-metrics counts
cargo test -p xt-metrics -p xt-fixtures
cargo clippy -p xt-metrics -p xt-fixtures --all-targets --locked -- -D warnings
cargo fmt --all --check
```

The counts tests cover precise window boundaries, session isolation, unknown
measurements, checked overflow, shared exclusions, copied-context invariance and
use of the timestamp index. The read-only open path prepares the actual count
query and rejects each missing measurement column without repairing the schema;
reopening the writer restores its projection. No new schema or storage path is
introduced.
