import { render, screen, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { expect, vi } from 'vitest';
import fixture from '../../fixtures/F1.json';
import { DataProvider } from '../data/DataProvider';
import type { DataSource, Unsubscribe } from '../data/DataSource';
import type { FixtureExport } from '../data/generated/FixtureExport';
import type { PrList } from '../data/generated/PrList';
import type { DataEvent } from '../data/ipc-names';
import { ThemeProvider } from '../theme/ThemeProvider';
import { AppRoutes } from './AppRoutes';

/** Shared by the pull requests page's test files only; nothing here ships. */

// JSON imports widen literal unions; the export is the generated shape.
export const exported = fixture as FixtureExport;

export const tokensByHost = async (days: number) => {
  const report = exported.dashboards.find((entry) => entry.window.days === days)!;
  return { window: report.window, hosts: report.tokens_by_host };
};

/**
 * A native-shaped source that answers the Shell's three reads (the app info,
 * the recorded tokens and the local index status its sidebar observes) and
 * this page's one, and traps everything else: a GitHub refresh or its cancel,
 * any session, transcript, Dashboard or Environment read, and the database
 * read no part of this route needs. A trapped call fails the read that made
 * it and is counted, so a test can also state that none happened. The status
 * answered is the export's, so no local history is read, and it is counted
 * too, so a test states exactly how often it was read.
 */
export function trappedSource(read: () => Promise<PrList>) {
  const trap = (name: string) =>
    vi.fn(async (): Promise<never> => {
      throw new Error(`${name} is not a read the pull requests page may make`);
    });
  return {
    kind: 'native' as const,
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: vi.fn(async () => exported.app_info),
    tokensByHost: vi.fn(tokensByHost),
    pullRequests: vi.fn(read),
    pullRequestAnalytics: async () => Promise.reject(new Error('not used by this test')),
    pullRequestSessions: async () => Promise.reject(new Error('not used by this test')),
    subscribe: vi.fn<DataSource['subscribe']>(async () => () => {}),
    refreshPullRequests: trap('refreshPullRequests'),
    cancelPullRequestRefresh: trap('cancelPullRequestRefresh'),
    dashboard: trap('dashboard'),
    environment: trap('environment'),
    today: trap('today'),
    sessionsList: trap('sessionsList'),
    sessionRow: trap('sessionRow'),
    sessionStretches: trap('sessionStretches'),
    sessionTranscript: trap('sessionTranscript'),
    cancelSessionTranscript: trap('cancelSessionTranscript'),
    dbCounts: trap('dbCounts'),
    // The sidebar's own read of the shared status query, never this page's.
    nativeIndexStatus: vi.fn(async () => structuredClone(exported.native_index)),
  } satisfies DataSource;
}
export type TrappedSource = ReturnType<typeof trappedSource>;

/**
 * Every trapped seam stayed untouched, and the local index status was read
 * exactly as often as the Shell's sidebar reads it: once when the app opened
 * on this route, where no page reads it, plus once per reconnect catch-up.
 * The count is exact, so a status read, poll or refetch of the page's own
 * fails here as a trapped call does.
 */
export function expectLocalReadsOnly(source: TrappedSource, statusReads = 1) {
  for (const trap of [
    source.refreshPullRequests,
    source.cancelPullRequestRefresh,
    source.dashboard,
    source.environment,
    source.today,
    source.sessionsList,
    source.sessionRow,
    source.sessionStretches,
    source.sessionTranscript,
    source.cancelSessionTranscript,
    source.dbCounts,
  ])
    expect(trap).not.toHaveBeenCalled();
  expect(source.nativeIndexStatus).toHaveBeenCalledTimes(statusReads);
}

/** The whole app at a route, so the Shell around the page is the real one. */
export const mount = (source: DataSource, path = '/prs?view=inventory') =>
  render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );

const TABLE = 'Linked pull requests';
export const table = () => screen.getByRole('table', { name: TABLE });
/** Every body row's cells, as text; the header row is left out. */
export const cells = () =>
  within(table())
    .getAllByRole('row')
    .slice(1)
    .map((row) =>
      within(row)
        .getAllByRole('cell')
        .map((cell) => cell.textContent),
    );
/** The first column of every body row. */
export const names = () => cells().map((row) => row[0]);
/** The runtime opens its screens once its listeners register; then the rows arrive. */
export const loaded = async (text: string | RegExp) =>
  within(await screen.findByRole('table', { name: TABLE })).findByText(text);
export const meta = () => document.querySelector('.xt-section-meta')!.textContent;

/** The page's own format, restated so an expectation does not depend on the machine's zone. */
export const shown = (ms: number) =>
  new Date(ms).toLocaleString(undefined, {
    month: 'short',
    day: 'numeric',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    hourCycle: 'h23',
  });

export const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { resolve, reject, promise };
};

/**
 * Replaces a source's `subscribe` with registrations the test settles, so an
 * attempt to hear events can be failed, and a later one completed, on demand.
 * Only a settled registration hears what `emit` sends.
 */
export function controlEvents(source: { subscribe: DataSource['subscribe'] }) {
  type Registration = {
    event: DataEvent;
    listener: (payload?: unknown) => void;
    settled: boolean;
    resolve(): void;
    reject(): void;
  };
  const registrations: Registration[] = [];
  const heard = new Set<Registration>();
  source.subscribe = vi.fn((event: DataEvent, listener: (payload?: unknown) => void) => {
    const done = deferred<Unsubscribe>();
    const registration: Registration = {
      event,
      listener,
      settled: false,
      resolve() {
        if (registration.settled) return;
        registration.settled = true;
        heard.add(registration);
        done.resolve(() => void heard.delete(registration));
      },
      reject() {
        if (registration.settled) return;
        registration.settled = true;
        done.reject(new Error('adapter detail: listen rejected'));
      },
    };
    registrations.push(registration);
    return done.promise;
  });
  return {
    registrations,
    /** Settles every registration not yet settled. */
    resolveAll: () => registrations.filter((r) => !r.settled).forEach((r) => r.resolve()),
    emit: (event: DataEvent, payload?: unknown) =>
      [...heard].filter((r) => r.event === event).forEach((r) => r.listener(payload)),
    heard: () => heard.size,
  };
}
