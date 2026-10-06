import type { SessionPage } from './generated/SessionPage';
import type { SessionRow } from './generated/SessionRow';
import type { SessionSourceStatus } from './generated/SessionSourceStatus';
import type { MetricSessionStretches } from './generated/MetricSessionStretches';
import type { DataSource, PullRequestSessionsRequest, SessionListFilter } from './DataSource';
import type { FixtureExport } from './generated/FixtureExport';
import type { PrList } from './generated/PrList';
import type { PrRefreshReport } from './generated/PrRefreshReport';
import type { PrRefreshRow } from './generated/PrRefreshRow';
import { events, type DataEvent } from './ipc-names';
import type { AccountUsage } from './generated/AccountUsage';

/**
 * The counters of a refresh report, derived from its rows exactly as the Rust
 * batch derives them: execution and persistence are counted separately, so an
 * attempt whose result storage would not keep is still attempted. A test
 * checks this against the exported report for the whole selection, so the two
 * cannot drift apart.
 */
export function refreshReport(rows: PrRefreshRow[], cancelled: boolean): PrRefreshReport {
  const count = (predicate: (row: PrRefreshRow) => boolean) => rows.filter(predicate).length;
  const succeeded = count((row) => row.outcome.outcome === 'succeeded');
  const failed = count((row) => row.outcome.outcome === 'failed');
  const skipped = count((row) => row.outcome.outcome === 'skipped');
  return {
    requested: succeeded + failed + skipped,
    attempted: succeeded + failed,
    succeeded,
    failed,
    skipped,
    unrecorded: count(
      (row) =>
        row.outcome.outcome !== 'skipped' && row.outcome.persistence.persistence !== 'recorded',
    ),
    cancelled,
    committed: rows.some(
      (row) =>
        row.outcome.outcome !== 'skipped' &&
        row.outcome.persistence.persistence === 'recorded' &&
        row.outcome.persistence.write === 'applied',
    ),
    rows,
  };
}

/** Imported only behind createDataSource's compile-time DEV boundary. */
export class FixtureDataSource implements DataSource {
  readonly kind = 'fixture';
  /** Browser fixtures never read a signed-in account or simulate a quota. */
  async accountUsage(): Promise<AccountUsage> {
    const unavailable = {
      state: 'unavailable',
      issue: 'source_unavailable',
      checked_at: null,
      windows: [],
    } as const;
    return { claude: { ...unavailable, windows: [] }, codex: { ...unavailable, windows: [] } };
  }
  /** Explicit fixture refresh is still offline and reports no signed-in account. */
  refreshClaudeUsage() {
    return this.accountUsage();
  }
  private listeners = new Map<DataEvent, Set<(payload?: unknown) => void>>();
  /** Pull requests this preview has already refreshed. */
  private refreshed = new Set<number>();
  constructor(private readonly fixture: FixtureExport) {}
  /**
   * Native fixture mode has no native home, so its rule activity service has
   * no source: the export carries that exact answer, read through the same
   * service. It is never a read of this machine's own rulebook, and never
   * an empty snapshot.
   */
  readonly ruleActivity = {
    read: async () => structuredClone(this.fixture.rule_activity),
    /** Nothing was started, so there is nothing to abandon. */
    cancel: async () => {},
  };
  private report(windowDays: number) {
    const report = this.fixture.dashboards.find((entry) => entry.window.days === windowDays);
    if (!report) throw new Error('Metric range must be 7, 14, or 30 days.');
    return report;
  }
  /**
   * The exported Dashboard, read before any refresh as a fixture database
   * starts. Once this preview has refreshed some listed pull requests, its
   * M-19 section is the one the export read after refreshing exactly those, on
   * a fresh database, through the application's own batch and assembler. A
   * state the export does not hold fails the read rather than showing numbers
   * this preview would have to make up.
   */
  async dashboard(windowDays: number) {
    const report = structuredClone(this.report(windowDays));
    const refreshed = this.refreshedIds();
    if (refreshed.length === 0) return report;
    const section = this.refreshedState(refreshed)?.sections.find(
      (entry) => entry.days === windowDays,
    );
    if (!section) throw new Error('The fixture export holds no M-19 report for this refresh.');
    report.tiles.merged_prs = structuredClone(section.merged_prs);
    report.pr_effort = structuredClone(section.pr_effort);
    return report;
  }
  /** The listed pull requests this preview has refreshed, in stored ID order. */
  private refreshedIds() {
    return this.fixture.pull_requests.rows
      .map((row) => row.pull_request.id)
      .filter((id) => this.refreshed.has(id))
      .sort((a, b) => a - b);
  }
  /** The export's state after refreshing exactly `refreshed`, from the start. */
  private refreshedState(refreshed: number[]) {
    return this.fixture.pr_effort_states.find(
      (entry) =>
        entry.refreshed.length === refreshed.length &&
        entry.refreshed.every((reference, index) => reference.id === refreshed[index]),
    );
  }
  /**
   * The PRs page report the export read for this range and confidence mode,
   * in the cached state this preview is in: before any refresh, or after
   * refreshing exactly what it refreshed. Every value is the Rust core's; a
   * combination the export does not hold fails rather than being computed.
   */
  async pullRequestAnalytics(windowDays: number, confirmedOnly: boolean) {
    const refreshed = this.refreshedIds();
    const pages =
      refreshed.length === 0
        ? this.fixture.pr_analytics
        : this.refreshedState(refreshed)?.analytics;
    const page = pages?.find(
      (entry) => entry.window.days === windowDays && entry.report.confirmed_only === confirmedOnly,
    );
    if (!page) throw new Error('The fixture export holds no PR report for this range and mode.');
    return structuredClone(page);
  }
  /**
   * The drilldown page the export read through the native command for exactly
   * this pull request, mode, range and report anchor. Membership is links,
   * which a refresh never changes; each link's title is the current PR state's,
   * as in the Sessions list. The fixture holds one page, and a request it did
   * not export fails rather than being filtered here.
   */
  async pullRequestSessions(
    { repository, number, confirmedOnly, windowDays, windowEndMs }: PullRequestSessionsRequest,
    after: string | null,
  ): Promise<SessionPage> {
    if (after) throw new Error('The fixture has one page.');
    const entry = this.fixture.pr_sessions.find(
      (candidate) =>
        // GitHub names are case-insensitive; the native identity is lowercase.
        candidate.repository === repository.toLowerCase() &&
        candidate.number === number &&
        candidate.confirmed_only === confirmedOnly &&
        candidate.window_days === windowDays &&
        candidate.window_end_ms === windowEndMs,
    );
    if (!entry) throw new Error('The fixture export holds no linked sessions for this request.');
    return { ...structuredClone(entry.page), rows: this.currentLinks(entry.page.rows) };
  }
  async environment(windowDays: number) {
    const report = this.fixture.environments.find((entry) => entry.window.days === windowDays);
    if (!report) throw new Error('Metric range must be 7, 14, or 30 days.');
    return structuredClone(report);
  }
  async today() {
    return structuredClone(this.fixture.today);
  }
  /**
   * The detail the export read through the native command for exactly this
   * span of this session. A span the fixture's lanes did not return fails
   * rather than being measured here.
   */
  readonly spanDetails = {
    read: async (sessionId: string, startMs: number, endMs: number) => {
      const entry = this.fixture.span_details.find(
        (candidate) =>
          candidate.session_id === sessionId &&
          candidate.start_ms === startMs &&
          candidate.end_ms === endMs,
      );
      if (!entry) throw new Error('The fixture export holds no detail for this span.');
      return structuredClone(entry.detail);
    },
    /** Nothing was started, so there is nothing to abandon. */
    cancel: async () => {},
  };
  async tokensByHost(windowDays: number) {
    const report = this.report(windowDays);
    return structuredClone({ window: report.window, hosts: report.tokens_by_host });
  }
  /**
   * The fixture's one page under the same filter the store applies: host set,
   * recorded pull-request links, and a case-insensitive substring of the ID,
   * repository, branch or saved title.
   */
  async sessionsList(
    { search, hosts, withPrs }: SessionListFilter,
    after: string | null,
    windowDays: number,
  ): Promise<SessionPage> {
    if (after) throw new Error('The fixture has one page.');
    const page = this.fixture.sessions.find((entry) => entry.window.days === windowDays);
    if (!page) throw new Error('Metric range must be 7, 14, or 30 days.');
    const needle = search.toLowerCase();
    const rows = page.rows.filter(
      (row) =>
        (!hosts || hosts.includes(row.host)) &&
        (!withPrs || row.pr_links.length > 0) &&
        [row.id, row.repo, row.branch, row.title].some((value) =>
          (value ?? '').toLowerCase().includes(needle),
        ),
    );
    return {
      window: page.window,
      rows: this.currentLinks(rows),
      next: null,
      referenced_parents: this.parentContext(page, rows),
    };
  }
  /**
   * Context only for each direct parent the kept rows name that they do not
   * list themselves, as the native page reads it: from the export's own
   * context, or from the parent's exported row when the filter dropped it.
   * Matched by exact identity and host; no further ancestor is read.
   */
  private parentContext(page: SessionPage, rows: readonly SessionRow[]) {
    const kept = new Set(rows.map((row) => row.id));
    const named = new Map<string, string>();
    for (const row of rows)
      if (row.parent && !kept.has(row.parent.session_id))
        named.set(row.parent.session_id, row.parent.host);
    const context: NonNullable<SessionPage['referenced_parents']> = [];
    for (const [id, host] of [...named].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))) {
      const sent = page.referenced_parents?.find(
        (entry) => entry.session_id === id && entry.host === host,
      );
      const listed = page.rows.find((row) => row.id === id && row.host === host);
      if (sent) context.push(structuredClone(sent));
      else if (listed)
        context.push({
          session_id: listed.id,
          host: listed.host,
          known_child: listed.known_child === true,
          parent: structuredClone(listed.parent ?? null),
          child_check: listed.child_check ?? null,
        });
    }
    return context;
  }
  /**
   * Rows whose pull-request link titles are this preview's current PR state.
   * A link's title is refresh-owned, and the export's session pages are read
   * before any refresh; so each title is the one {@link pullRequests} holds
   * right now for the same canonical identity, exactly as the native list
   * joins the same stored row. Nothing is computed; titles are only looked up.
   */
  private currentLinks(rows: readonly SessionRow[]): SessionRow[] {
    const titles = new Map(
      this.currentPullRequests().map((row) => [
        `${row.pull_request.repository}#${row.pull_request.number}`,
        row.title,
      ]),
    );
    return structuredClone(rows).map((row) => ({
      ...row,
      pr_links: row.pr_links.map((link) => {
        const key = `${link.repository}#${link.number}`;
        return titles.has(key) ? { ...link, title: titles.get(key) ?? null } : link;
      }),
    }));
  }
  /**
   * The fixture's own row for exactly this identity, over the same window the
   * list would measure it in — the identity is compared exactly, as the native
   * command compares it, so a prefix of another session is not this one.
   */
  async sessionRow(sessionId: string, windowDays: number) {
    const page = this.fixture.sessions.find((entry) => entry.window.days === windowDays);
    if (!page) throw new Error('Metric range must be 7, 14, or 30 days.');
    const row = page.rows.find((candidate) => candidate.id === sessionId);
    return row ? this.currentLinks([row])[0] : null;
  }
  /**
   * The stretches the fixture export carries for exactly this identity and
   * window — the same command the native app answers, run against the fixture
   * database at export time. An identity the fixture does not list is
   * `missing`, as it is natively: no indexed session owns it.
   */
  async sessionStretches(sessionId: string, windowDays: number): Promise<MetricSessionStretches> {
    if (!this.fixture.sessions.some((page) => page.window.days === windowDays))
      throw new Error('Metric range must be 7, 14, or 30 days.');
    const entry = this.fixture.session_stretches.find(
      (candidate) => candidate.window_days === windowDays && candidate.session_id === sessionId,
    );
    return entry ? structuredClone(entry.stretches) : { state: 'missing' };
  }
  /**
   * A fixture is a database, not a history: it carries measurements taken
   * from sessions whose original files this machine does not have. So there
   * is no local source to open, and the honest answer is the same one the
   * app gives when it has no native home — not an empty transcript, which
   * would say the session recorded nothing.
   */
  async sessionTranscript(): Promise<SessionSourceStatus> {
    return { state: 'unavailable', reason: { reason: 'not_indexed' } };
  }
  /** Nothing was started, so there is nothing to abandon. */
  async cancelSessionTranscript() {}
  async appInfo() {
    return structuredClone(this.fixture.app_info);
  }
  async dbCounts() {
    return structuredClone(this.fixture.db_counts);
  }
  async nativeIndexStatus() {
    return structuredClone(this.fixture.native_index);
  }
  /**
   * The stored pull requests, with the rows this preview has already
   * refreshed replaced by what the exported refresh left behind. Every value
   * is the shared Rust export's: no GitHub CLI is resolved or run here, no
   * credential is read and no request is made.
   */
  async pullRequests(): Promise<PrList> {
    return structuredClone({ rows: this.currentPullRequests() });
  }
  /** The shared lookup behind {@link pullRequests} and the list's link titles. */
  private currentPullRequests(): PrList['rows'] {
    const refreshed = new Map(
      this.fixture.pull_requests_refreshed.rows.map((row) => [row.pull_request.id, row]),
    );
    return this.fixture.pull_requests.rows.map((row) =>
      this.refreshed.has(row.pull_request.id) ? (refreshed.get(row.pull_request.id) ?? row) : row,
    );
  }
  /**
   * The exported outcome of each selected pull request. An identifier the
   * export does not hold is reported unstored, exactly as the native command
   * reports one storage does not hold. The native command owns the selection
   * rules; this only refuses an empty one.
   */
  async refreshPullRequests(ids: number[]): Promise<PrRefreshReport> {
    if (ids.length === 0) throw new Error('Select at least one pull request to refresh.');
    const exported = new Map(this.fixture.pr_refresh.rows.map((row) => [row.id, row]));
    const rows = ids.map((id): PrRefreshRow => {
      const row = exported.get(id);
      if (!row)
        return { id, pull_request: null, outcome: { outcome: 'skipped', reason: 'not_stored' } };
      // Native fixture mode stamps every attempt with the fixture's pinned
      // instant, so refreshing a pull request again stores the very same
      // result at the very same time: storage reports it unchanged, nothing
      // commits and no event is emitted. The preview does the same.
      const outcome = structuredClone(row.outcome);
      if (
        this.refreshed.has(id) &&
        outcome.outcome !== 'skipped' &&
        outcome.persistence.persistence === 'recorded'
      )
        outcome.persistence.write = 'unchanged';
      return { ...row, outcome };
    });
    for (const row of rows) if (row.outcome.outcome !== 'skipped') this.refreshed.add(row.id);
    const report = structuredClone(refreshReport(rows, false));
    // The app emits the refresh event only after a committed change; so does
    // this, through the same event map.
    if (report.committed) this.emit(events.prsRefreshed);
    return report;
  }
  /** A fixture refresh is already over by the time it returns. */
  async cancelPullRequestRefresh() {
    return false;
  }
  async subscribe(event: DataEvent, listener: (payload?: unknown) => void) {
    const listeners = this.listeners.get(event) ?? new Set();
    listeners.add(listener);
    this.listeners.set(event, listeners);
    return () => {
      listeners.delete(listener);
    };
  }
  /** Deterministic event source for development/test harnesses, not production IPC. */
  emit(event: DataEvent, payload?: unknown) {
    this.listeners.get(event)?.forEach((listener) => listener(payload));
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
