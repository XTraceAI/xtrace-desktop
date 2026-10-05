import type { EnvIdentityRow } from '../../data/generated/EnvIdentityRow';
import type { EnvironmentMetrics } from '../../data/generated/EnvironmentMetrics';
import type { MetricIdentityCalls } from '../../data/generated/MetricIdentityCalls';
import type { MetricSurfaceCalls } from '../../data/generated/MetricSurfaceCalls';
import type { MetricToolIdentity } from '../../data/generated/MetricToolIdentity';

/**
 * Test-only synthetic Environment report: not real history, not a design sample and not the
 * planned 58-item inventory. It replaces the observed usage of a generated F1 report (keeping its
 * disclosed windows, strip dates and configured facts) with eleven identities in the order Rust
 * would assign, one strip of each shade step (0, 2, 5 and 9 calls), hook summaries with unresolved
 * attribution, unknown kind and details, a future kind and a raw future surface. Seven codex
 * observations have no timestamp: as in Rust, every window reports them as unresolved, so
 * `selected_unresolved_calls` counts them while `selected_calls` (the surfaces) never does.
 */
type Spec = {
  host: string;
  surface: string | null;
  identity: MetricToolIdentity;
  calls: number;
  strip: number[];
};
const id = (
  kind: string | null,
  name: string,
  detail: Partial<Pick<MetricToolIdentity, 'server' | 'tool' | 'skill'>> = {},
): MetricToolIdentity => ({ kind, name, server: null, tool: null, skill: null, ...detail });
const quiet = (last: number) => [...Array<number>(13).fill(0), last];

export const syntheticSpecs: readonly Spec[] = [
  {
    host: 'claude',
    surface: 'cli',
    identity: id('skill', 'Bash', { skill: 'Bash' }),
    calls: 42,
    strip: [0, 2, 5, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  },
  {
    host: 'claude',
    surface: 'cli',
    identity: id('hook', 'stop_hook_summary'),
    calls: 20,
    strip: quiet(4),
  },
  {
    host: 'claude',
    surface: 'cli',
    identity: id('mcp', 'mcp__memhub__search_memory', { server: 'memhub', tool: 'search_memory' }),
    calls: 12,
    strip: quiet(3),
  },
  {
    host: 'codex',
    surface: 'future.app',
    identity: id('skill', 'shell', { skill: 'shell' }),
    calls: 9,
    strip: quiet(2),
  },
  { host: 'claude', surface: 'cli', identity: id('skill', 'Skill'), calls: 6, strip: quiet(1) },
  {
    host: 'cursor',
    surface: null,
    identity: id('future_kind', 'Composer'),
    calls: 5,
    strip: quiet(1),
  },
  { host: 'claude', surface: 'cli', identity: id(null, 'LegacyTool'), calls: 4, strip: quiet(1) },
  { host: 'claude', surface: 'cli', identity: id('command', 'review'), calls: 3, strip: quiet(1) },
  { host: 'claude', surface: 'cli', identity: id('subagent', 'Task'), calls: 2, strip: quiet(1) },
  {
    host: 'claude',
    surface: 'cli',
    identity: id('mcp', 'mcp__broken'),
    calls: 1,
    strip: quiet(1),
  },
  // Called only earlier in the fixed strip, not in the selected 7 days.
  {
    host: 'codex',
    surface: 'cli',
    identity: id('skill', 'apply_patch', { skill: 'apply_patch' }),
    calls: 0,
    strip: [3, ...Array<number>(13).fill(0)],
  },
];

const sum = (values: readonly number[]) => values.reduce((total, value) => total + value, 0);
/** Untimed observations belong to no window, so every report carries the same count. */
export const SYNTHETIC_UNTIMED = 7;
/** Selected calls grow with the range; the fixed 14-day strip does not. */
const scale = { 7: 1, 14: 2, 30: 4 } as Record<number, number>;

/** F1 has no visible tool rows after built-ins are excluded. Its UTC strip window still
 * supplies the date labels for synthetic reports used only by UI tests. */
const stripDates = (base: EnvironmentMetrics) =>
  Array.from({ length: 14 }, (_, index) => {
    const start_ms = base.strip_window.start_ms + index * 86_400_000;
    const end_ms = Math.min(start_ms + 86_400_000, base.strip_window.end_ms);
    return { date: new Date(start_ms).toISOString().slice(0, 10), start_ms, end_ms, calls: 0 };
  });

/** One synthetic named skill keeps the compact-row UI checks meaningful when F1
 * itself has only filtered-out built-in calls. */
export function singleSkillEnvironment(base: EnvironmentMetrics): EnvironmentMetrics {
  const report = structuredClone(base);
  const identity = id('skill', 'Review', { skill: 'Review' });
  const strip = stripDates(base);
  strip[13].calls = 5;
  const row: EnvIdentityRow = {
    order: 0,
    host: 'claude',
    identity,
    calls: 5,
    strip_calls: 5,
    strip,
  };
  const observed = {
    host: 'claude',
    surface: 'cli',
    calls: 5,
    by_day: [],
    by_identity: [{ identity, calls: 5, by_day: [] }],
  };
  report.identities = [row];
  report.selected.observed = [structuredClone(observed)];
  report.strip.observed = [structuredClone(observed)];
  report.totals = { ...report.totals, selected_calls: 5, strip_calls: 5, identities: 1 };
  return report;
}

export function syntheticEnvironment(base: EnvironmentMetrics): EnvironmentMetrics {
  const report = structuredClone(base);
  const factor = scale[report.window.days] ?? 1;
  const dates = stripDates(base);
  const identities: EnvIdentityRow[] = syntheticSpecs.map((spec, order) => ({
    order,
    host: spec.host,
    identity: spec.identity,
    calls: spec.calls * factor,
    strip_calls: sum(spec.strip),
    strip: dates.map((day, index) => ({ ...day, calls: spec.strip[index] })),
  }));
  // Day series are not read by the panel; the synthetic surfaces carry none.
  const observed = (pick: (spec: Spec, row: EnvIdentityRow) => number) => {
    const surfaces: MetricSurfaceCalls[] = [];
    syntheticSpecs.forEach((spec, index) => {
      const calls = pick(spec, identities[index]);
      if (calls === 0) return;
      let surface = surfaces.find(
        (item) => item.host === spec.host && item.surface === spec.surface,
      );
      if (!surface) {
        surface = { host: spec.host, surface: spec.surface, calls: 0, by_day: [], by_identity: [] };
        surfaces.push(surface);
      }
      surface.calls += calls;
      const item: MetricIdentityCalls = { identity: spec.identity, calls, by_day: [] };
      surface.by_identity.push(item);
    });
    return surfaces;
  };
  const unresolved = (calls: (index: number) => number) => [
    ...(
      [
        ['hook_attribution', 1],
        ['unknown_skill_name', 4],
        ['unknown_kind', 6],
        ['unknown_mcp_detail', 9],
      ] as const
    ).map(([reason, index]) => ({ host: 'claude', reason, calls: calls(index) })),
    { host: 'codex', reason: 'missing_timestamp' as const, calls: SYNTHETIC_UNTIMED },
  ];
  const hostUnresolved = (usage: EnvironmentMetrics['selected'], host: string) =>
    sum(usage.unresolved.filter((item) => item.host === host).map((item) => item.calls));
  report.identities = identities;
  report.selected.observed = observed((_, row) => row.calls);
  report.selected.unresolved = unresolved((index) => identities[index].calls);
  report.strip.observed = observed((_, row) => row.strip_calls);
  report.strip.unresolved = unresolved((index) => identities[index].strip_calls);
  for (const usage of [report.selected, report.strip])
    usage.hosts = usage.hosts.map((host) => ({
      ...host,
      unresolved_calls: hostUnresolved(usage, host.host),
    }));
  report.totals = {
    ...report.totals,
    selected_calls: sum(identities.map((row) => row.calls)),
    strip_calls: sum(identities.map((row) => row.strip_calls)),
    selected_unresolved_calls: sum(report.selected.unresolved.map((item) => item.calls)),
    identities: identities.length,
  };
  return report;
}

/**
 * A synthetic report shaped like the counts root saw on a real local index
 * (13,558 observed calls, 48 identities, 4,913 unresolved observations,
 * including untimed ones) or larger — not copied from it. Every row is one
 * of the eleven synthetic templates above with a computed share of `calls`,
 * distinct in name, and every total is the sum of what it counts, as Rust
 * would report it: `calls` over the identities, `unresolved` over the reason
 * groups (the untimed codex group keeps its fixed count), `identities` over
 * the rows. The strips keep the templates' fixed 14 days.
 */
export function shapedEnvironment(
  base: EnvironmentMetrics,
  shape: { calls: number; identities: number; unresolved: number },
): EnvironmentMetrics {
  const report = structuredClone(base);
  const dates = stripDates(base);
  // Descending shares that add up to `calls` exactly: the first row takes the rounding.
  const weight = (index: number) => shape.identities - index;
  const weights = sum(Array.from({ length: shape.identities }, (_, index) => weight(index)));
  const shares = Array.from({ length: shape.identities }, (_, index) =>
    Math.floor((shape.calls * weight(index)) / weights),
  );
  shares[0] += shape.calls - sum(shares);
  const rows: EnvIdentityRow[] = shares.map((calls, order) => {
    const spec = syntheticSpecs[order % syntheticSpecs.length];
    const name = `${spec.identity.name}-${order + 1}`;
    return {
      order,
      host: spec.host,
      identity: {
        ...spec.identity,
        name,
        skill: spec.identity.kind === 'skill' ? name : spec.identity.skill,
      },
      calls,
      strip_calls: sum(spec.strip),
      strip: dates.map((day, index) => ({ ...day, calls: spec.strip[index] })),
    };
  });
  const surfaces = (pick: (row: EnvIdentityRow) => number) => {
    const out: MetricSurfaceCalls[] = [];
    rows.forEach((row, index) => {
      const spec = syntheticSpecs[index % syntheticSpecs.length];
      const calls = pick(row);
      if (calls === 0) return;
      let surface = out.find((item) => item.host === spec.host && item.surface === spec.surface);
      if (!surface) {
        surface = { host: spec.host, surface: spec.surface, calls: 0, by_day: [], by_identity: [] };
        out.push(surface);
      }
      surface.calls += calls;
      surface.by_identity.push({ identity: row.identity, calls, by_day: [] });
    });
    return out;
  };
  // Four timed reason groups share `unresolved` less the untimed group, the first taking the rounding.
  const timed = shape.unresolved - SYNTHETIC_UNTIMED;
  const groups = [
    'hook_attribution',
    'unknown_skill_name',
    'unknown_kind',
    'unknown_mcp_detail',
  ] as const;
  const perGroup = groups.map(() => Math.floor(timed / groups.length));
  perGroup[0] += timed - sum(perGroup);
  const unresolved = [
    ...groups.map((reason, index) => ({ host: 'claude', reason, calls: perGroup[index] })),
    { host: 'codex', reason: 'missing_timestamp' as const, calls: SYNTHETIC_UNTIMED },
  ];
  const hostUnresolved = (host: string) =>
    sum(unresolved.filter((item) => item.host === host).map((item) => item.calls));
  report.identities = rows;
  report.selected.observed = surfaces((row) => row.calls);
  report.selected.unresolved = unresolved;
  report.strip.observed = surfaces((row) => row.strip_calls);
  report.strip.unresolved = unresolved;
  for (const usage of [report.selected, report.strip])
    usage.hosts = usage.hosts.map((host) => ({
      ...host,
      unresolved_calls: hostUnresolved(host.host),
    }));
  report.totals = {
    ...report.totals,
    selected_calls: sum(rows.map((row) => row.calls)),
    strip_calls: sum(rows.map((row) => row.strip_calls)),
    selected_unresolved_calls: sum(unresolved.map((item) => item.calls)),
    identities: rows.length,
  };
  return report;
}
