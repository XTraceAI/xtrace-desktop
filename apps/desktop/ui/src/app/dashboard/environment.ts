import type { EnvCacheSource } from '../../data/generated/EnvCacheSource';
import type { EnvComponentKind } from '../../data/generated/EnvComponentKind';
import type { EnvConfigSource } from '../../data/generated/EnvConfigSource';
import type { EnvIdentityRow } from '../../data/generated/EnvIdentityRow';
import type { EnvironmentMetrics } from '../../data/generated/EnvironmentMetrics';
import type { EnvRootState } from '../../data/generated/EnvRootState';
import type { EnvSourceStatus } from '../../data/generated/EnvSourceStatus';
import type { MetricToolIdentity } from '../../data/generated/MetricToolIdentity';
import type { MetricUnresolvedReason } from '../../data/generated/MetricUnresolvedReason';
import { calendarDay } from '../../kit/clock';
import type { ControlTone } from '../../kit/control-tone';
import { plural, surfaceLabel } from './present';

/**
 * Presentation only. Rows, totals and day buckets come from the Rust bridge in its order; nothing
 * here sums, sorts or infers installation. The inventory is unknown, so nothing may claim a
 * component is unused, missing or not installed.
 */

/** The dashboard shows this many identities; the rest are one keyboard step away. */
export const TOP_IDENTITIES = 8;
/** Strip shade steps: 0, fewer than 3, fewer than 8, then 8 or more calls per day. */
export const STRIP_THRESHOLDS = [3, 8] as const;
/** The stored name of a stop-hook summary; it states that hooks ran, not which one. */
const HOOK_SUMMARY = 'stop_hook_summary';

const kindTone: Record<string, ControlTone> = {
  builtin: 'meta',
  mcp: 'info',
  skill: 'accent',
  hook: 'success',
  command: 'warning',
  subagent: 'accent',
};
/** The stored kind verbatim; an unrecorded or future kind stays visible, never dropped. */
export const kindBadge = (kind: string | null) =>
  kind === null
    ? { text: 'unknown kind', tone: 'warning' as const }
    : { text: kind, tone: kindTone[kind] ?? ('meta' as const) };

export const isHookSummary = (identity: MetricToolIdentity) =>
  identity.kind === 'hook' && identity.name === HOOK_SUMMARY;

/** A truthful display name and, when a detail was never stated, what is missing. */
export function identityText(identity: MetricToolIdentity): { name: string; note?: string } {
  const { kind, name, server, tool, skill } = identity;
  if (isHookSummary(identity))
    return { name: 'Hook summary events', note: 'name absent from index' };
  if (kind === null) return { name, note: 'kind not recorded' };
  if (kind === 'mcp')
    return server !== null && tool !== null
      ? { name: `${server} · ${tool}` }
      : { name, note: 'MCP server and tool not stated' };
  if (kind === 'skill')
    return skill !== null ? { name: skill } : { name, note: 'skill name not stated' };
  return { name };
}

/** Hook rows count summary events, never calls of a named hook. */
export const callsText = (identity: MetricToolIdentity, calls: number) =>
  identity.kind === 'hook' ? plural(calls, 'event') : plural(calls, 'call');

const sameIdentity = (a: MetricToolIdentity, b: MetricToolIdentity) =>
  a.kind === b.kind &&
  a.name === b.name &&
  a.server === b.server &&
  a.tool === b.tool &&
  a.skill === b.skill;

/**
 * The raw surfaces this row's calls were observed on, looked up (not counted) in the selected
 * report, else the strip report for a row with calls only in the strip.
 */
export function rowSurfaces(report: EnvironmentMetrics, row: EnvIdentityRow): string[] {
  const find = (usage: EnvironmentMetrics['selected']) =>
    usage.observed
      .filter(
        (surface) =>
          surface.host === row.host &&
          surface.by_identity.some(
            (item) => item.calls > 0 && sameIdentity(item.identity, row.identity),
          ),
      )
      .map((surface) => surfaceLabel(surface.host, surface.surface));
  const selected = find(report.selected);
  return selected.length > 0 ? selected : find(report.strip);
}

/** A local calendar date (`YYYY-MM-DD`) as a label; the date is already local to the report. */
export const dayLabel = (date: string) => calendarDay(date);

/** What each unresolved group is; none of these calls can be credited to a named component. */
export const unresolvedText: Record<MetricUnresolvedReason, (calls: number) => string> = {
  hook_attribution: (calls) =>
    `${plural(calls, 'hook summary event')}: a summary says hooks ran, not which one`,
  unknown_kind: (calls) => `${plural(calls, 'call')}: kind not recorded`,
  unknown_mcp_detail: (calls) => `${plural(calls, 'MCP call')}: server and tool not stated`,
  unknown_skill_name: (calls) => `${plural(calls, 'skill call')}: skill name not stated`,
  missing_timestamp: (calls) =>
    `${plural(calls, 'untimed observation')}: no timestamp, so no selected range can hold it`,
};

/**
 * A missing timestamp is counted apart: those observations fall in no window, so they are not part
 * of the selected range's observed calls, and no unresolved figure is a fraction of those calls.
 */
export const isUntimed = (reason: MetricUnresolvedReason) => reason === 'missing_timestamp';

export const configSourceText: Record<EnvConfigSource, string> = {
  claude_skill_directory: 'Claude skills directory',
  claude_user_mcp_config: 'Claude user MCP config',
  claude_project_mcp_config: 'Claude project MCP config',
  claude_plugin_registry: 'Claude plugin registry',
  claude_enabled_plugins: 'Claude plugin enablement settings',
  claude_hook_config: 'Claude hook settings',
  codex_mcp_config: 'Codex MCP config',
  cursor_mcp_config: 'Cursor MCP config',
};

export const cacheSourceText: Record<EnvCacheSource, string> = {
  codex_plugin_cache: 'Codex plugin cache',
  cursor_plugin_cache: 'Cursor plugin cache',
};

export const componentKindText: Record<EnvComponentKind, string> = {
  mcp_server: 'MCP server',
  skill: 'skill',
  plugin: 'plugin',
  hook: 'hook entry',
};

export const enabledText = (enabled: boolean | null) =>
  enabled === null ? 'enablement not stated' : enabled ? 'enabled' : 'disabled';

/** Scope and 1-based position of the supplied root; never a path. */
export const scopeText = (scope: 'home' | 'repository', index?: number) =>
  index === undefined ? scope : `${scope} ${index + 1}`;

/**
 * A directory listing stops one entry past the bound, so an entry-limit count from a directory is a
 * lower bound; a file states its full count even when it is over the bound.
 */
const directorySources: ReadonlySet<EnvConfigSource | EnvCacheSource> = new Set([
  'claude_skill_directory',
  'codex_plugin_cache',
  'cursor_plugin_cache',
]);

export function statusText(
  status: EnvSourceStatus,
  source: EnvConfigSource | EnvCacheSource,
): string {
  if (
    status.state === 'incomplete' &&
    status.reason === 'entry_limit' &&
    directorySources.has(source)
  )
    return `incomplete · at least ${plural(status.stated, 'entry', 'entries')}, at least ${
      status.skipped
    } skipped (over the entry limit; the listing stopped, so both are lower bounds)`;
  switch (status.state) {
    case 'missing':
      return 'not present';
    case 'empty':
      return 'present, states nothing';
    case 'read':
      return `read · ${status.stated} stated`;
    case 'incomplete':
      return `incomplete · ${status.stated} stated, ${status.skipped} skipped (${
        status.reason === 'entry_limit'
          ? 'over the entry limit'
          : 'entries without the documented shape'
      })`;
    case 'malformed':
      return 'could not be parsed';
    case 'unreadable':
      return 'could not be read';
    case 'unsupported':
      return `not read · ${
        {
          symlink: 'linked paths are not followed',
          not_regular_file: 'not a regular file',
          not_directory: 'not a directory',
          file_too_large: 'over the size limit',
          unsupported_schema: 'unsupported format version',
        }[status.reason]
      }`;
  }
}

export const rootStateText: Record<EnvRootState, string> = {
  read: 'read',
  missing: 'not present',
  not_directory: 'not a directory',
  symlink: 'linked path not followed',
  unreadable: 'could not be read',
};

/** A source that was present but not fully read; components may exist that it did not verify. */
const unverified = (status: EnvSourceStatus) =>
  ['incomplete', 'malformed', 'unreadable', 'unsupported'].includes(status.state);

/**
 * An empty configured list is what was verified, never a statement that nothing is configured:
 * a source that could not be fully read may name components this did not verify.
 */
export function configuredEmptyText(sources: EnvironmentMetrics['sources']): string {
  const partial = sources.filter((item) => unverified(item.status)).length;
  return partial > 0
    ? `No configured components were verified. ${plural(partial, 'source was', 'sources were')} not fully read, so components may be configured that this does not show.`
    : 'No configured components were verified. No supported source that was read states one.';
}
