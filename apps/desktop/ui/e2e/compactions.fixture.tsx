import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import fixture from '../fixtures/F1.json';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { CompactionCount } from '../src/data/generated/CompactionCount';
import { DataProvider } from '../src/data/DataProvider';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import { StartupSplash } from '../src/kit/StartupSplash';
import { AppRoutes } from '../src/app/AppRoutes';
import '../src/index.css';

const exported = structuredClone(fixture as FixtureExport);
const base = exported.sessions[0].rows[0];
const readDelay = new URLSearchParams(location.search).get('readDelay') === '600' ? 600 : 0;
// Eight displayed rows barely overflow the dashboard. The delayed-scroll case
// needs enough rows to move its tracked session entirely outside the scroller.
const rows = Array.from({ length: readDelay ? 32 : 9 }, (_, index) => ({
  ...base,
  host: 'codex',
  id: `compaction-${index}`,
  child_check: index === 8 ? ('child' as const) : ('checked' as const),
  known_child: index === 8,
  title: `Synthetic session ${index} with a deliberately long title to check the narrow window`,
  parent:
    index === 8
      ? {
          session_id: 'compaction-5',
          host: 'codex',
          title: 'Synthetic parent',
          evidence: 'native_spawn' as const,
        }
      : null,
}));
for (const page of exported.sessions) {
  page.rows = rows;
  page.next = null;
}
for (const report of exported.dashboards) {
  report.lanes = rows.map((row, index) => ({
    session_id: row.id,
    host: 'codex',
    start_ms: report.lane_start_ms + index * 60000,
    end_ms: report.lane_end_ms - index * 60000,
  }));
  report.lane_sessions = rows.map((row) => ({
    session_id: row.id,
    host: 'codex',
    title: row.title,
    repo: row.repo,
    branch: row.branch,
    parent: row.parent,
    child_check: row.child_check,
    known_child: row.known_child,
    automated_review: false,
    started_at_ms: null,
    cost: null,
    pr_links: 0,
    inferred_pr_links: 0,
  }));
}
// Counts 1–5 carry one recorded time per compaction, spread across the lane
// window with each trigger; the count of 9 carries none, as Cursor's do.
// Sessions 2 and 3 are forks: 3 inherited none, 2's inherited part is unknown.
const triggers = ['auto', 'manual', 'unknown'] as const;
const { lane_start_ms: laneStart, lane_end_ms: laneEnd } = exported.dashboards[0];
const timed = (count: number) =>
  Array.from({ length: count }, (_, index) => ({
    at_ms: Math.round(laneStart + ((index + 1) / (count + 1)) * (laneEnd - laneStart)),
    trigger: triggers[index % 3],
  }));
// The scroll regression holds source answers long enough to observe a new
// visible batch. Other fixture checks keep their immediate answers.
const delayRead = async () => {
  if (readDelay) await new Promise((resolve) => setTimeout(resolve, readDelay));
};
const source = Object.assign(new FixtureDataSource(exported), {
  liveSessions: {
    read: async (ids: readonly string[], viewId: string | null) => {
      await delayRead();
      return {
        view_id: viewId ?? 'synthetic-live-lease',
        states: ids.map((id) => ({ id, status: 'running' as const })),
      };
    },
    release: async () => {},
  },
  compactions: {
    read: async (ids: readonly string[]) => {
      await delayRead();
      return {
        counts: ids.map((id) => {
          const n = Number(id.split('-').at(-1));
          const outcome: CompactionCount =
            n === 6
              ? { state: 'count', count: 9, events: [] }
              : n === 7
                ? { state: 'unknown', reason: 'ownership' }
                : n === 2
                  ? {
                      state: 'count',
                      count: n,
                      events: timed(n),
                      inherited: { state: 'unknown', reason: 'missing' },
                    }
                  : n === 3
                    ? {
                        state: 'count',
                        count: n,
                        events: timed(n),
                        inherited: { state: 'count', count: 0 },
                      }
                    : { state: 'count', count: n, events: timed(n) };
          return { id, outcome };
        }),
      };
    },
    cancel: async () => {},
  },
});
const requestedView = new URLSearchParams(location.search).get('view');
const view = requestedView === 'dashboard' ? '/dashboard' : '/sessions';
createRoot(document.getElementById('root')!).render(
  <ThemeProvider>
    {requestedView === 'startup' ? (
      <StartupSplash>Opening XTrace…</StartupSplash>
    ) : (
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[view]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    )}
  </ThemeProvider>,
);
