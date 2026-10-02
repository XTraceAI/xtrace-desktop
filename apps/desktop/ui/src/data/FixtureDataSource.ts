import type { SessionPage } from './generated/SessionPage';
import type { DataSource } from './DataSource';
import type { FixtureExport } from './generated/FixtureExport';
import type { DataEvent } from './ipc-names';

/** Imported only behind createDataSource's compile-time DEV boundary. */
export class FixtureDataSource implements DataSource {
  readonly kind = 'fixture';
  private listeners = new Map<DataEvent, Set<() => void>>();
  constructor(private readonly fixture: FixtureExport) {}
  private report(windowDays: number) {
    const report = this.fixture.dashboards.find((entry) => entry.window.days === windowDays);
    if (!report) throw new Error('Metric range must be 7, 14, or 30 days.');
    return report;
  }
  async dashboard(windowDays: number) {
    return structuredClone(this.report(windowDays));
  }
  async tokensByHost(windowDays: number) {
    const report = this.report(windowDays);
    return structuredClone({ window: report.window, hosts: report.tokens_by_host });
  }
  async sessionsList(
    search: string,
    host: string | null,
    _after: string | null,
  ): Promise<SessionPage> {
    if (_after) throw new Error('The fixture has one page.');
    const rows = this.fixture.sessions.rows.filter(
      (row) =>
        (!host || row.host === host) &&
        [row.id, row.repo, row.branch].some((value) =>
          value?.toLowerCase().includes(search.toLowerCase()),
        ),
    );
    return { rows: structuredClone(rows), next: null };
  }
  async appInfo() {
    return structuredClone(this.fixture.app_info);
  }
  async dbCounts() {
    return structuredClone(this.fixture.db_counts);
  }
  async nativeIndexStatus() {
    return structuredClone(this.fixture.native_index);
  }
  async subscribe(event: DataEvent, listener: () => void) {
    const listeners = this.listeners.get(event) ?? new Set();
    listeners.add(listener);
    this.listeners.set(event, listeners);
    return () => {
      listeners.delete(listener);
    };
  }
  /** Deterministic event source for development/test harnesses, not production IPC. */
  emit(event: DataEvent) {
    this.listeners.get(event)?.forEach((listener) => listener());
  }
}

export async function loadFixtureDataSource(id: string): Promise<FixtureDataSource> {
  if (!import.meta.env.DEV || id !== 'F1')
    throw new Error('Unsupported fixture. Only F1 is implemented.');
  const exports = import.meta.glob<FixtureExport>('../../fixtures/*.json', { import: 'default' });
  const load = exports['../../fixtures/F1.json'];
  if (!load) throw new Error('The F1 export is unavailable.');
  const fixture = await load();
  if (fixture.app_info.fixture !== id) throw new Error('The fixture export identity differs.');
  return new FixtureDataSource(fixture);
}
