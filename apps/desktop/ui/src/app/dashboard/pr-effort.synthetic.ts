import type { DashboardMetrics } from '../../data/generated/DashboardMetrics';
import type { MetricEffortAssignment } from '../../data/generated/MetricEffortAssignment';
import type { MetricEffortDay } from '../../data/generated/MetricEffortDay';
import type { MetricEffortTotals } from '../../data/generated/MetricEffortTotals';
import type { MetricMergedPrs } from '../../data/generated/MetricMergedPrs';
import type { MetricModelDayEffort } from '../../data/generated/MetricModelDayEffort';
import type { MetricEffortCost } from '../../data/generated/MetricEffortCost';
import type { MetricPrFreshness } from '../../data/generated/MetricPrFreshness';
import type { MetricPrMarker } from '../../data/generated/MetricPrMarker';
import type { MetricTile } from '../../data/generated/MetricTile';

/**
 * Test-only synthetic M-19 sections: not real history and not a design sample.
 * Each replaces the M-19 section of a generated F1 report, keeping its window
 * and local days, with values shaped the way the Rust report shapes them:
 * disjoint assignments in Rust's order that add up to the cohort, a dollar
 * total that is unknown (never zero) once any selected response is unpriced,
 * with the priced subtotal and the unpriced responses named beside it, and a
 * tile whose count is unknown while any retained pull request lacks cached
 * facts. Each day's models add up to the day as Rust's do: the priced part
 * under the day's model, unpriced responses under the unpriced model, and the
 * agent time under the day's model.
 */
type DaySpec = {
  agentMs?: number;
  /** The priced subtotal in USD. */
  usd?: number;
  selected?: number;
  priced?: number;
  /** Priced Codex responses that recorded no tier. */
  assumed?: number;
  /** The model of the day's priced responses and agent time; `null` is no model recorded. */
  model?: string | null;
};
type GroupSpec = {
  assignment: MetricEffortAssignment;
  sessions: number;
  days: Record<number, DaySpec>;
};
type MarkerSpec = {
  number: number;
  day: number;
  workType: string | null;
  confidence?: MetricPrMarker['confidence'];
  freshness?: MetricPrFreshness;
};
export type SectionSpec = {
  tile: Partial<MetricMergedPrs>;
  previous?: Partial<MetricMergedPrs>;
  markers?: MarkerSpec[];
  groups: GroupSpec[];
};

/** Unpriced responses are a model the catalog does not price. */
const UNPRICED_MODEL = 'unpublished-model';

const cost = (spec: DaySpec | undefined): MetricEffortCost => {
  const selected = spec?.selected ?? (spec?.usd === undefined ? 0 : 1);
  const priced = spec?.priced ?? selected;
  const unpriced = selected - priced;
  const subtotal = spec?.usd ?? 0;
  return {
    selected_observations: selected,
    priced_observations: priced,
    unpriced_observations: unpriced,
    assumed_tier_observations: spec?.assumed ?? 0,
    total_usd: selected > 0 && unpriced === 0 ? subtotal : null,
    priced_subtotal_usd: subtotal,
    unpriced:
      unpriced > 0
        ? [
            {
              model: UNPRICED_MODEL,
              service_tier: null,
              reason: 'unknown_model',
              observations: unpriced,
            },
          ]
        : [],
  };
};
const addCost = (a: MetricEffortCost, b: MetricEffortCost): MetricEffortCost => {
  const selected = a.selected_observations + b.selected_observations;
  const unpriced = a.unpriced_observations + b.unpriced_observations;
  const subtotal = a.priced_subtotal_usd + b.priced_subtotal_usd;
  return {
    selected_observations: selected,
    priced_observations: a.priced_observations + b.priced_observations,
    unpriced_observations: unpriced,
    assumed_tier_observations: a.assumed_tier_observations + b.assumed_tier_observations,
    total_usd: selected > 0 && unpriced === 0 ? subtotal : null,
    priced_subtotal_usd: subtotal,
    unpriced:
      unpriced > 0
        ? [
            {
              model: UNPRICED_MODEL,
              service_tier: null,
              reason: 'unknown_model',
              observations: unpriced,
            },
          ]
        : [],
  };
};
const none: MetricEffortCost = cost(undefined);

/** The model a day spec names when it names none. */
export const SYNTHETIC_MODEL = 'synthetic-model';
const NANO = 1_000_000_000;

/** One day spec's models, in Rust's name order (`null` first). */
const models = (spec: DaySpec | undefined): MetricModelDayEffort[] => {
  if (!spec) return [];
  const day = cost(spec);
  const name = spec.model === undefined ? SYNTHETIC_MODEL : spec.model;
  const out = new Map<string | null, MetricModelDayEffort>();
  const entry = (model: string | null) => {
    const found = out.get(model);
    if (found) return found;
    const fresh = {
      model,
      priced_nano_usd: 0,
      priced_observations: 0,
      unpriced_observations: 0,
      agent_ms: 0,
    };
    out.set(model, fresh);
    return fresh;
  };
  if (day.priced_observations > 0) {
    const priced = entry(name);
    priced.priced_observations = day.priced_observations;
    priced.priced_nano_usd = Math.round(day.priced_subtotal_usd * NANO);
  }
  if (day.unpriced_observations > 0)
    entry(UNPRICED_MODEL).unpriced_observations = day.unpriced_observations;
  if ((spec.agentMs ?? 0) > 0) entry(name).agent_ms = spec.agentMs!;
  return sortModels([...out.values()]);
};
const sortModels = (list: MetricModelDayEffort[]) =>
  list.sort((a, b) =>
    a.model === b.model
      ? 0
      : a.model === null
        ? -1
        : b.model === null
          ? 1
          : a.model < b.model
            ? -1
            : 1,
  );
const addModels = (a: MetricModelDayEffort[], b: MetricModelDayEffort[]) => {
  const out = new Map(a.map((model) => [model.model, { ...model }]));
  for (const model of b) {
    const found = out.get(model.model);
    if (!found) out.set(model.model, { ...model });
    else {
      found.priced_nano_usd += model.priced_nano_usd;
      found.priced_observations += model.priced_observations;
      found.unpriced_observations += model.unpriced_observations;
      found.agent_ms += model.agent_ms;
    }
  }
  return sortModels([...out.values()]);
};

const tileOf = (patch: Partial<MetricMergedPrs>): MetricMergedPrs => {
  const known = patch.known_merged ?? 0;
  const unknown = patch.unknown_facts ?? 0;
  return {
    known_merged: known,
    unknown_facts: unknown,
    complete: unknown === 0,
    merged: unknown === 0 ? known : null,
    unresolved_type: 0,
    freshness: {
      never_attempted: unknown,
      refreshed: known,
      failed_never_refreshed: 0,
      failed_after_refresh: 0,
      manual_failed_never_refreshed: 0,
      manual_failed_after_refresh: 0,
      oldest_refreshed_at: known > 0 ? 1_788_825_600_000 : null,
      newest_attempted_at: known > 0 ? 1_788_825_600_000 : null,
    },
    ...patch,
  };
};

/** The merged-PR tile exactly as the Rust assembler states the core tile. */
export function mergedTile(current: MetricMergedPrs, previous: MetricMergedPrs): MetricTile {
  return {
    value: current.merged,
    unit: 'prs',
    rule_id: 'M-19',
    reason:
      current.merged === null
        ? `${current.known_merged} merged so far; not known yet for ${current.unknown_facts} linked pull ${
            current.unknown_facts === 1 ? 'request' : 'requests'
          }`
        : null,
    note: 'Counts pull requests your sessions are linked to by an exact or commit link; guessed links are left out. Merge facts are read from GitHub with the gh CLI.',
    current_n: current.merged,
    previous_n: previous.merged,
    sample_unit: 'prs',
    delta: { previous: previous.merged, pct: null, suppressed: true },
  };
}

export function withPrEffort(base: DashboardMetrics, spec: SectionSpec): DashboardMetrics {
  const report = structuredClone(base);
  const frame = report.pr_effort.current.cohort.by_day;
  const totals = (days: MetricEffortDay[], sessions: number): MetricEffortTotals => ({
    sessions,
    cost: days.reduce((sum, day) => addCost(sum, day.cost), none),
    agent_ms: days.reduce((sum, day) => sum + day.agent_ms, 0),
    by_day: days,
  });
  const by_assignment = spec.groups.map((group) => ({
    assignment: group.assignment,
    effort: totals(
      frame.map((day, index) => ({
        date: day.date,
        start_ms: day.start_ms,
        end_ms: day.end_ms,
        cost: cost(group.days[index]),
        agent_ms: group.days[index]?.agentMs ?? 0,
        models: models(group.days[index]),
      })),
      group.sessions,
    ),
  }));
  const cohortDays = frame.map((day, index) => ({
    ...day,
    cost: by_assignment.reduce(
      (sum, group) => addCost(sum, group.effort.by_day[index]!.cost),
      none,
    ),
    agent_ms: by_assignment.reduce((sum, group) => sum + group.effort.by_day[index]!.agent_ms, 0),
    models: by_assignment.reduce<MetricModelDayEffort[]>(
      (sum, group) => addModels(sum, group.effort.by_day[index]!.models),
      [],
    ),
  }));
  const current = tileOf(spec.tile);
  const previous = tileOf(spec.previous ?? {});
  report.pr_effort = {
    rule_id: 'M-19',
    current: {
      confirmed_only: true,
      tile: current,
      markers: (spec.markers ?? []).map((marker) => {
        const day = frame[marker.day]!;
        return {
          repository: 'xtrace/app',
          number: marker.number,
          url: `https://github.com/xtrace/app/pull/${marker.number}`,
          merged_at: new Date(day.start_ms + 43_200_000).toISOString(),
          merged_at_ms: day.start_ms + 43_200_000,
          date: day.date,
          work_type: marker.workType,
          confidence: marker.confidence ?? 'exact',
          freshness: marker.freshness ?? { state: 'refreshed' },
        };
      }),
      by_assignment,
      cohort: totals(
        cohortDays,
        by_assignment.reduce((sum, group) => sum + group.effort.sessions, 0),
      ),
    },
    previous,
  };
  report.tiles.merged_prs = mergedTile(current, previous);
  return report;
}

const ty = (work_type: string): MetricEffortAssignment => ({ kind: 'type', work_type });
const HALF_HOUR = 1_800_000;

/** F19: one $2.50 session linked exactly to a feat and a fix PR. */
export const F19_SHARED: SectionSpec = {
  tile: { known_merged: 2 },
  markers: [
    { number: 1, day: 4, workType: 'feat' },
    { number: 2, day: 5, workType: 'fix' },
  ],
  groups: [
    {
      assignment: { kind: 'mixed' },
      sessions: 1,
      days: { 2: { agentMs: HALF_HOUR, usd: 2.5 } },
    },
  ],
};

/** F19's inferred variant after confirmed-only: the fix link is gone before assignment. */
export const F19_CONFIRMED: SectionSpec = {
  tile: { known_merged: 1 },
  markers: [{ number: 1, day: 4, workType: 'feat' }],
  groups: [{ assignment: ty('feat'), sessions: 1, days: { 2: { agentMs: HALF_HOUR, usd: 2.5 } } }],
};

/** One known merged chore PR and one linked PR with no cached facts. */
export const UNKNOWN_FACTS: SectionSpec = {
  tile: { known_merged: 1, unknown_facts: 1 },
  markers: [{ number: 1, day: 4, workType: 'chore' }],
  groups: [
    { assignment: ty('chore'), sessions: 1, days: { 2: { agentMs: HALF_HOUR, usd: 0.4 } } },
    {
      assignment: { kind: 'unresolved' },
      sessions: 1,
      days: { 3: { agentMs: HALF_HOUR, usd: 0.8 } },
    },
  ],
};

/** No session links a pull request: a measured zero and only `other` work. */
export const NO_LINKS: SectionSpec = {
  tile: { known_merged: 0 },
  groups: [
    { assignment: { kind: 'other' }, sessions: 2, days: { 1: { agentMs: HALF_HOUR, usd: 0.28 } } },
  ],
};

/** A day where one of two selected responses could not be priced. */
export const PARTIAL_COST: SectionSpec = {
  tile: { known_merged: 1 },
  markers: [{ number: 1, day: 4, workType: 'feat' }],
  groups: [
    {
      assignment: ty('feat'),
      sessions: 1,
      days: { 2: { agentMs: HALF_HOUR, usd: 1.25, selected: 2, priced: 1, assumed: 1 } },
    },
  ],
};

/** Canonical types as classified, beyond any three sample categories. */
export const MANY_TYPES: SectionSpec = {
  tile: { known_merged: 5 },
  markers: [
    { number: 1, day: 1, workType: 'docs' },
    { number: 2, day: 2, workType: 'perf' },
    { number: 3, day: 2, workType: 'refactor' },
    { number: 4, day: 3, workType: 'feat', confidence: 'sha' },
    {
      number: 5,
      day: 3,
      workType: 'fix',
      freshness: { state: 'failed_after_refresh', error: 'rate_limited' },
    },
  ],
  groups: [
    { assignment: ty('docs'), sessions: 1, days: { 1: { agentMs: 600_000, usd: 0.2 } } },
    { assignment: ty('perf'), sessions: 1, days: { 2: { agentMs: 600_000, usd: 0.2 } } },
    { assignment: ty('refactor'), sessions: 1, days: { 2: { agentMs: 600_000, usd: 0.2 } } },
    { assignment: { kind: 'mixed' }, sessions: 1, days: { 3: { agentMs: 600_000, usd: 0.2 } } },
    { assignment: { kind: 'other' }, sessions: 1, days: { 4: { agentMs: 600_000, usd: 0.2 } } },
  ],
};
