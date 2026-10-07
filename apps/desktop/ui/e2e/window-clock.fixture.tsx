import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import fixture from '../fixtures/F1.json';
import { AppRoutes } from '../src/app/AppRoutes';
import { DataProvider } from '../src/data/DataProvider';
import type { DataSource } from '../src/data/DataSource';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { SessionPage } from '../src/data/generated/SessionPage';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

/**
 * The app's own Shell and routes over the F1 fixture, opened at `?path=`.
 * Every data read is counted by method on `window.__reads`, so the spec can
 * say which reads the clock began and which it left alone.
 */
const path = new URLSearchParams(location.search).get('path') ?? '/dashboard';
const reads: Record<string, number> = {};
Object.assign(window, { __reads: reads });
const exported = fixture as FixtureExport;
const source = new FixtureDataSource(exported);
// Three synthetic pages exercise the mounted list's loaded-page replay and
// scroll position; the normal F1 fixture keeps its original one-page data.
// With `groups`, five first-page sessions are sub-sessions of the last
// session on the third page, so their group's parent arrives late.
if (new URLSearchParams(location.search).get('pages') === '3') {
  const held = new URLSearchParams(location.search).get('hold');
  const groups = new URLSearchParams(location.search).has('groups');
  const PARENT = 'clock-session-2-49';
  const first = exported.sessions.find((entry) => entry.window.days === 7)!;
  const row = first.rows[0];
  let pageReads = 0;
  Object.assign(window, { __holdNextList: false });
  Object.defineProperty(source, 'sessionsList', {
    configurable: true,
    value: async (_filter: unknown, after: string | null): Promise<SessionPage> => {
      pageReads += 1;
      const control = window as unknown as { __holdNextList: boolean; __releaseList?: () => void };
      if (held && control.__holdNextList) {
        control.__holdNextList = false;
        await new Promise<void>((resolve) => {
          control.__releaseList = resolve;
        });
      }
      const page = after === null ? 0 : Number(after);
      const parent = {
        session_id: PARENT,
        host: row.host,
        title: 'Clock parent',
        evidence: 'native_spawn' as const,
      };
      const rows = Array.from({ length: 50 }, (_, index) => {
        const id = `clock-session-${page}-${String(index).padStart(2, '0')}`;
        const child = groups && page === 0 && index >= 10 && index < 15;
        return {
          ...row,
          id,
          title: groups && id === PARENT ? 'Clock parent' : row.title,
          parent: child ? parent : null,
          known_child: child,
          child_check: child ? ('child' as const) : ('checked' as const),
        };
      });
      if (pageReads > 3 && page === 2) rows.reverse();
      return {
        window: first.window,
        rows,
        next: page < 2 ? String(page + 1) : null,
        referenced_parents:
          groups && page === 0
            ? [
                {
                  session_id: PARENT,
                  host: row.host,
                  known_child: false,
                  parent: null,
                  child_check: 'checked',
                },
              ]
            : [],
      };
    },
  });
}
const counted = [
  'dashboard',
  'tokensByHost',
  'environment',
  'today',
  'sessionsList',
  'sessionRow',
  'sessionStretches',
  'sessionTranscript',
  'pullRequests',
  'pullRequestAnalytics',
  'pullRequestSessions',
  'appInfo',
  'dbCounts',
  'nativeIndexStatus',
] as const satisfies readonly (keyof DataSource)[];
for (const method of counted) {
  const read = (source[method] as (...args: unknown[]) => unknown).bind(source);
  reads[method] = 0;
  Object.defineProperty(source, method, {
    value: (...args: unknown[]) => {
      reads[method] += 1;
      return read(...args);
    },
  });
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  </StrictMode>,
);
