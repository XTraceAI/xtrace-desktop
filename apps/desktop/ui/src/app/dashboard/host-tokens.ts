import type { TokensByHost } from '../../data/generated/TokensByHost';
import type { HostTokens } from '../../kit/Sidebar';

const sidebarHosts = ['claude', 'codex', 'cursor'] as const;
const isSidebarHost = (host: string): host is HostTokens['host'] =>
  (sidebarHosts as readonly string[]).includes(host);

/**
 * Recorded token totals from the report, per host. The bar is each host's share of the
 * measured totals shown, not a quota, limit or remaining balance.
 */
export function hostTokenRows(report: TokensByHost | undefined): HostTokens[] {
  const rows = (report?.hosts ?? [])
    .filter((row) => isSidebarHost(row.host))
    .map((row) => ({
      host: row.host as HostTokens['host'],
      tokens: row.tokens.counters.total_tokens,
    }));
  const recorded = rows.reduce((sum, row) => sum + (row.tokens ?? 0), 0);
  return rows.map((row) => ({
    ...row,
    fillPercent: recorded > 0 && row.tokens !== null ? (row.tokens / recorded) * 100 : 0,
  }));
}
