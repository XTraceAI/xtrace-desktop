import type { EnvIdentityRow } from '../../data/generated/EnvIdentityRow';
import type { EnvironmentMetrics } from '../../data/generated/EnvironmentMetrics';
import type { MetricIdentityCalls } from '../../data/generated/MetricIdentityCalls';
import type { MetricSurfaceCalls } from '../../data/generated/MetricSurfaceCalls';
import type { MetricToolIdentity } from '../../data/generated/MetricToolIdentity';

/**
 * Test-only synthetic Environment report with eleven identities. It replaces the observed usage
 * of a generated F1 report (keeping its
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
    identity: id('builtin', 'Bash'),
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
    identity: id('builtin', 'shell'),
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
    identity: id('builtin', 'apply_patch'),
    calls: 0,
    strip: [3, ...Array<number>(13).fill(0)],
  },
];

const sum = (values: readonly number[]) => values.reduce((total, value) => total + value, 0);
/** Untimed observations belong to no window, so every report carries the same count. */
export const SYNTHETIC_UNTIMED = 7;
/** Selected calls grow with the range; the fixed 14-day strip does not. */
const scale = { 7: 1, 14: 2, 30: 4 } as Record<number, number>;

export function syntheticEnvironment(base: EnvironmentMetrics): EnvironmentMetrics {
  const report = structuredClone(base);
  const factor = scale[report.window.days] ?? 1;
  const dates = base.identities[0].strip;
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
