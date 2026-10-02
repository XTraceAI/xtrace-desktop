# API-equivalent token cost

`MetricsDb::cost(window, zone, &PriceCatalog)` prices currently selected response
observations from one indexed read query. Every row is priced once; total, host,
recorded model, raw surface and local-day breakdowns share that result. Exact
native instants determine half-open window membership and day placement. Existing
response selection, work exclusions and UUID ownership remain authoritative.

The DTO and catalog state this basis:

> Global public token API-equivalent, using recorded service tier; excludes unrecorded regional/speed modifiers, tool fees, and subscriptions.

This is not an invoice. `price_version` and `price_as_of` identify the supplied
catalog. `total_usd` is null if any selected observation is unpriced, or when no
usage observation exists. `priced_subtotal_usd` is a separately named partial
subtotal, never a substitute for an unknown total. Each unpriced group names the
recorded model, service tier, reason and observation count. Independently measured
tokens remain available through the token report.

## Catalog and arithmetic

The bundled `prices.json` is validated before use. Rates are integer
`nano_usd_per_token`; $1 per million tokens equals 1,000 nano-USD per token. Checked
`u128` multiplication and accumulation continue through every aggregate, followed
by one final conversion to USD. There is no intermediate cent rounding.

Validation rejects unknown fields, malformed dates, blank/duplicate model aliases
and tier identities, unordered or nonterminal context bands, negative/fractional
rates, and incompatible cache-write modes. Null rates mean unavailable, never
zero. The catalog accepts only exact model/alias/tier names. It has no file-path
loader, network fetch, price table, stored record-cost cache or update worker.

Fresh input, output, cache reads and cache writes are disjoint canonical counters.
Codex output already includes reasoning; informational reasoning counters are not
priced again. Claude cache writes use separate five-minute and one-hour rates.
Positive writes require both measured TTL counters whose sum equals total writes.
Zero writes need no absent TTL breakdown, but a contradictory nonzero breakdown
is unpriced. Missing model, service tier, rate, counters, prompt-threshold facts,
or cache split produces a named unpriced reason.

Subsequent reads naturally reflect selected-row replacement, usage/tier enrichment
and a different validated catalog object, without mutating historical records.
Rejected catalogs cannot replace a previously validated object.

## Public sources and supported identities

The bundled version is `public-token-usd-2026-09-17`, verified on 2026-09-17 UTC.
Prices below are USD per million tokens at short-context standard rates:

| Exact model               | Fresh input | Cache read | Flat write | 5m write | 1h write | Output |
| ------------------------- | ----------: | ---------: | ---------: | -------: | -------: | -----: |
| gpt-6-astra               |          10 |          1 |       12.5 |        — |        — |     50 |
| gpt-5.6-sol               |           4 |        0.4 |          5 |        — |        — |     20 |
| gpt-5.6-terra             |           2 |        0.2 |        2.5 |        — |        — |     12 |
| gpt-5.6-luna              |         0.2 |       0.02 |       0.25 |        — |        — |    1.2 |
| claude-opus-5             |           5 |        0.5 |          — |     6.25 |       10 |     25 |
| claude-sonnet-5           |           2 |        0.2 |          — |      2.5 |        4 |     10 |
| claude-haiku-4-5-20251001 |           1 |        0.1 |          — |     1.25 |        2 |      5 |

The [OpenAI pricing table](https://developers.openai.com/api/docs/pricing) provides
explicit input/read/write/output rates for both context bands and supported
processing modes. [Astra](https://developers.openai.com/api/docs/models/gpt-6-astra),
[Sol](https://developers.openai.com/api/docs/models/gpt-5.6-sol),
[Terra](https://developers.openai.com/api/docs/models/gpt-5.6-terra) and
[Luna](https://developers.openai.com/api/docs/models/gpt-5.6-luna) specify the
strictly-above-272,000-input-token threshold. Canonical fresh input plus cache reads
plus cache writes supplies that prompt total; output does not affect it. Long
context doubles input/cache rates and multiplies output by 1.5 for the entire
request. Sol explicitly documents the `gpt-5.6` alias; its current promotional
rates are available at least through November 21, 2026.

The [OpenAI response reference](https://developers.openai.com/api/reference/cli/resources/responses/methods/create)
identifies `default` as standard pricing and documents `flex` and the equivalent
`priority`/`fast` names. The catalog supports those exact tiers; Flex rates are
half and Fast rates twice the applicable standard rates. No `standard`, `auto`,
`scale`, `batch`, or `ultrafast` mapping is guessed. The pinned Codex reader does
not currently supply service tier, so those missing-tier observations remain
unpriced; this change does not add billing metadata to the reader.

The [Claude pricing page](https://platform.claude.com/docs/en/about-claude/pricing)
and [model overview](https://platform.claude.com/docs/en/models/overview) establish
the exact three Claude IDs and rates above. Claude 4.6 and later have standard
pricing across their full context; Haiku 4.5 has its documented 200K context.
[Batch result usage](https://platform.claude.com/docs/en/api/cli/messages/batches/results)
names `standard` and `batch`, and [batch processing](https://platform.claude.com/docs/en/build-with-claude/batch-processing)
confirms caching discounts stack with the 50% batch discount. Those two tiers are
supported. Contractual priority pricing, request-only `auto`/`standard_only`,
undocumented aliases and guessed date suffixes remain unpriced. Claude speed and
regional modifiers are outside the stated basis.

## Synthetic checks

The F1 named `prices` snapshot holds synthetic integer rates independently of the
production catalog. Arithmetic includes:

- Claude: 1,000 fresh, 100 output, 2,000 reads, 300 five-minute writes and 200
  one-hour writes cost $0.002975 at synthetic standard rates; the synthetic fast
  tier doubles that amount.
- Codex: 1,000 fresh, 300 reasoning-inclusive output, 2,000 reads and 500 writes
  cost $0.004825. Informational reasoning is not added again.
- Cursor: a documented synthetic alias resolves to $0.000263 for its test row.
- F17 replaces the earlier response cost and assigns its selected timestamp to
  the correct day/window. Its deliberately missing-model row keeps total cost
  unknown while the known subtotal is $0.000073.
- F18 usage enrichment changes cost with no new records and the same price
  version. Explicit synthetic service tiers supplement fixtures that omit tier;
  no real absent tier is assumed.

Tests cover the seven bundled IDs, verified tiers, context boundaries, zero
writes, missing/inconsistent splits, unpriced context, version changes, invalid
catalogs, overflow, precise/leap membership, read-side schema probes, indexed
candidate selection and concurrent-write snapshot consistency.

Run `cargo test -p xt-metrics cost` and
`cargo test -p xt-metrics -p xt-fixtures --locked`, followed by Clippy and formatting.
