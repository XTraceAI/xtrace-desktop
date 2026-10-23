import type { DashboardMetrics } from './generated/DashboardMetrics';
import type { TokensByHost } from './generated/TokensByHost';
import type { SessionPage } from './generated/SessionPage';
import type { SessionRow } from './generated/SessionRow';
import type { MetricSessionStretches } from './generated/MetricSessionStretches';
import type { AppInfo } from './generated/AppInfo';
import type { DbCounts } from './generated/DbCounts';
import type { NativeIndexStatus } from './generated/NativeIndexStatus';
import type { SessionSourceStatus } from './generated/SessionSourceStatus';
import type { PrList } from './generated/PrList';
import type { PrRefreshReport } from './generated/PrRefreshReport';
import type { PrAnalyticsPage } from './generated/PrAnalyticsPage';
import type { PrAutoCheckStatus } from './generated/PrAutoCheckStatus';
import type { DataEvent } from './ipc-names';
import type { ContentRetention } from './generated/ContentRetention';
import type { ContentPurge } from './generated/ContentPurge';
import type { TodaySummary } from './generated/TodaySummary';
import type { SessionTitles } from './generated/SessionTitles';
import type { SessionCompactions } from './generated/SessionCompactions';
import type { DashboardDaySessions } from './generated/DashboardDaySessions';
import type { DashboardSpanDetail } from './generated/DashboardSpanDetail';
import type { RuleActivityResult } from './generated/RuleActivityResult';
import type { ProjectList } from './generated/ProjectList';
import type { GradingSkills } from './generated/GradingSkills';
import type { GradingOverview } from './generated/GradingOverview';
import type { GradingRuns } from './generated/GradingRuns';
import type { GradingStart } from './generated/GradingStart';
import type { GradingInstalledSkills } from './generated/GradingInstalledSkills';
import type { GradingSkillAdd } from './generated/GradingSkillAdd';
import type { GradingSkillRemove } from './generated/GradingSkillRemove';
import type { GradingRubricMake } from './generated/GradingRubricMake';
import type { GradingLinkLookup } from './generated/GradingLinkLookup';
import type { GradingLinkKeep } from './generated/GradingLinkKeep';
import type { AccountUsage } from './generated/AccountUsage';
import type { LiveSessionSnapshot } from './generated/LiveSessionSnapshot';
import type { TelemetryState } from './generated/TelemetryState';

export type Unsubscribe = () => void;
/** Development builds only: release information without update installation. */
export interface LocalUpdateControls {
  /** Opens the fixed public releases page in the system browser, only on a user click. */
  viewPublicReleases(): Promise<void>;
}
/** Exact cached Codex Desktop status for the rows this view displays. */
export interface LiveSessionControls {
  /** Register an empty lease with ([], null), then read using its native-issued view_id. */
  read(sessionIds: readonly string[], viewId: string | null): Promise<LiveSessionSnapshot>;
  release(viewId: string): Promise<void>;
}
/**
 * Which indexed sessions the list names. Every part is applied by the store
 * before paging, so a filter never thins a page that was already cut.
 */
export type SessionSort = 'started' | 'recently_active';

export interface SessionListFilter {
  /** Orders the whole filtered list before paging; defaults to Started. */
  sort?: SessionSort;
  /**
   * Substring of the ID, repository, branch or a title the index saved; at
   * most 256 characters. A host title read transiently for display is not
   * indexed, so it is not searched.
   */
  search: string;
  /** `null` lists every host; otherwise a non-empty host set. */
  hosts: string[] | null;
  /** Only sessions with a recorded pull-request link, of any confidence. */
  withPrs: boolean;
}
/**
 * One pull request's linked-session drilldown: its canonical identity, the
 * confidence mode of the report it was opened from, and that report's preset
 * and `window.end_ms`. The anchor is the displayed report's, never a new clock
 * reading, so every page measures the report's own window.
 */
export interface PullRequestSessionsRequest {
  /** Canonical lowercase `owner/repo`, as the report row names it. */
  repository: string;
  number: number;
  confirmedOnly: boolean;
  windowDays: number;
  windowEndMs: number;
}
/** The app database's existing P-01 mode and P-02 purge; nothing is decided here. */
export interface RetentionControls {
  read(): Promise<ContentRetention>;
  /** Future imports and enrichment only; resolves with the mode read back after commit. */
  set(mode: ContentRetention): Promise<ContentRetention>;
  /** Call only after user confirmation; resolves with the committed outcome. */
  purge(): Promise<ContentPurge>;
}
/**
 * The saved typing speed the typing estimate reads, in whole words per minute.
 * The app database owns the bounds and the default; nothing is measured here.
 */
export interface TypingSpeedControls {
  /** The saved speed; 40 when nothing was saved. */
  read(): Promise<number>;
  /** A whole number from 1 to 300; resolves with the speed read back after commit. */
  set(wpm: number): Promise<number>;
  /** Opens the fixed official typing test in the system browser; no URL crosses this seam. */
  openTest(): Promise<void>;
}
/**
 * The saved break length "human time" read, in whole minutes: two messages
 * you sent at most this far apart join one stretch. The app database owns the
 * bounds and the default; nothing is measured here.
 */
export interface HumanBreakControls {
  /** The saved length; 60 when nothing was saved. */
  read(): Promise<number>;
  /** A whole number from 5 to 240; resolves with the length read back after commit. */
  set(minutes: number): Promise<number>;
}
/**
 * Anonymous usage telemetry. The app database owns the saved preference and
 * whether the disclosure was shown; the Rust core decides every field of the
 * state. Each call resolves with the state read back after it.
 */
export interface TelemetryControls {
  read(): Promise<TelemetryState>;
  /** Refused while `XTRACE_TELEMETRY` turns telemetry off. */
  setEnabled(enabled: boolean): Promise<TelemetryState>;
  /** Call only once the disclosure is actually visible on screen. */
  acknowledgeDisclosure(): Promise<TelemetryState>;
  /** Opens the fixed published event list in the system browser. */
  openEventList(): Promise<void>;
}
/**
 * Host titles for the rows a list is showing, read from the sessions' original
 * local sources on demand and never stored. Present only where there is a
 * local history to read.
 */
export interface SessionTitleControls {
  /**
   * The host's own title for each named session that has one: at most one
   * Sessions page of distinct canonical identifiers, and nothing else — which
   * file to read is resolved behind this seam. The answer is sparse; an absent
   * row keeps its identifier. `readId` is this read's own name for
   * {@link cancel}.
   */
  read(sessionIds: readonly string[], readId: string): Promise<SessionTitles>;
  /** Abandon a read; cancelling one that has not begun is not a mistake. */
  cancel(readId: string): Promise<void>;
}

export interface SessionCompactionControls {
  read(sessionIds: readonly string[], readId: string): Promise<SessionCompactions>;
  cancel(readId: string): Promise<void>;
}

/**
 * One activity-lane span's detail — its most-used tool, output tokens and the
 * last message a person typed in it or before it — named by the span's
 * session and its two endpoints exactly as the Dashboard report sent them.
 * Read only when a span's bubble opens; nothing is calculated here. When the
 * index kept no words, the native side reads them from the session's original
 * source in memory, so the read is cancellable like a transcript open.
 */
export interface SpanDetailControls {
  /** `readId` is this read's own name for {@link cancel}. */
  read(
    sessionId: string,
    startMs: number,
    endMs: number,
    readId: string,
  ): Promise<DashboardSpanDetail>;
  /** Abandon a read; cancelling one that has not begun is not a mistake. */
  cancel(readId: string): Promise<void>;
}

/**
 * Recorded rule activity: one bounded snapshot of the default local rulebook
 * over the fixed trailing 14 days, which the native side anchors when it
 * admits the read. Read only when a view asks — on opening or an explicit
 * refresh, never on an event, focus or timer.
 */
export interface RuleActivityControls {
  /**
   * `readId` is this read's own name for {@link cancel} and the only thing
   * sent: no root, path, range or filter. Every outcome, including `busy`
   * while another read runs, is a typed state; a view keeps only the answer
   * to the read it is still waiting for.
   */
  read(readId: string): Promise<RuleActivityResult>;
  /** Abandon a read; cancelling one that has not begun is not a mistake. */
  cancel(readId: string): Promise<void>;
}
/**
 * Grading runs: a grading skill (built in, or added from the skills the user
 * installed) run on one project, on request.
 * Every decision is native: which skills exist, whether a run may start (the
 * folder must be exactly a project a fresh project list names; one run at a
 * time), how it ended and its score. `grading://changed` follows every
 * committed status change and `grading://progress` reports elapsed seconds.
 * A rubric-making job shares the one slot.
 */
export interface GradingControls {
  skills(): Promise<GradingSkills>;
  /**
   * The Scores page's answer, decided natively over a fresh project list and
   * the stored runs: each skill's shown scores, average and top insights,
   * and the run going now. Reading starts nothing.
   */
  overview(): Promise<GradingOverview>;
  /** The recent runs, newest first. Reading starts nothing. */
  runs(): Promise<GradingRuns>;
  /** Sends only the project's path and the skill's ID. */
  start(projectPath: string, skillId: string): Promise<GradingStart>;
  /** False when that run is not the one going now. */
  cancel(runId: string): Promise<boolean>;
  /**
   * The skills installed on this Mac, each with its rubric status and
   * whether it can be added to Scores, decided natively. Reading writes
   * nothing.
   */
  installed(): Promise<GradingInstalledSkills>;
  /** Sends only the location the installed list named; native re-checks it. */
  addSkill(location: string): Promise<GradingSkillAdd>;
  /** Takes an added skill off Scores; its grades and files stay. */
  removeSkill(skillId: string): Promise<GradingSkillRemove>;
  /**
   * Starts making a rubric for the installed skill at `location` (one the
   * installed list named) with the bundled rubric maker, in the one grading
   * slot. `cancel(jobId)` stops it; `grading://rubric-progress` reports its
   * elapsed seconds and `grading://changed` follows its start, a saved
   * rubric and its end.
   */
  makeRubric(location: string): Promise<GradingRubricMake>;
  /**
   * "Add from a link": looks up a public GitHub link the user pasted (the
   * only network use of the dialog, from GitHub without signing in), and
   * answers the skills it holds with whether each can be kept, decided
   * natively. A new look-up replaces the previous one.
   */
  linkLookup(link: string): Promise<GradingLinkLookup>;
  /** Cancels the look-up going now; false when none is. */
  linkCancel(): Promise<boolean>;
  /**
   * Keeps the skills at `paths` (ones the preview named) in
   * `~/.agents/skills`, each linked in `~/.claude/skills` when that name is
   * free; native checks each again first. `grading://changed` follows.
   */
  linkKeep(lookupId: string, paths: string[]): Promise<GradingLinkKeep>;
  /** Drops a preview and its downloaded files. */
  linkDiscard(lookupId: string): Promise<boolean>;
}
/**
 * The automatic pull-request check. The native side runs it by itself on
 * window focus and about once an hour, only reading GitHub through the same
 * bounded batch as the manual refresh; `request` also asks it to look when
 * the Dashboard is shown. Neither call waits for a check to finish.
 */
export interface PrAutoCheckControls {
  status(): Promise<PrAutoCheckStatus>;
  request(): Promise<PrAutoCheckStatus>;
}
/** The menu-bar popover's own window; present only in that native window. */
export interface TrayControls {
  /** Whether the popover is on screen now, for a view mounted after a show. */
  visible(): Promise<boolean>;
  hide(): Promise<void>;
  /** Hide the popover and bring the existing main window forward. */
  openMain(): Promise<void>;
  onShown(listener: () => void): Promise<Unsubscribe>;
  onHidden(listener: () => void): Promise<Unsubscribe>;
}
/** Extend this seam with generated query DTOs when their owning screen lands. */
export interface DataSource {
  /** Confirmed native updater mode, selected before exposing update controls. */
  readonly publicUpdates?: true;
  readonly localUpdates?: LocalUpdateControls;
  readonly liveSessions?: LiveSessionControls;
  /** Provider-reported account windows, independent of local history ranges. */
  accountUsage(): Promise<AccountUsage>;
  /** Claude login is read only after the user presses Refresh in Usage. */
  refreshClaudeUsage(): Promise<AccountUsage>;
  dashboard(windowDays: number): Promise<DashboardMetrics>;
  tokensByHost(windowDays: number): Promise<TokensByHost>;
  /** Local midnight to one captured instant, read in one snapshot. */
  today(): Promise<TodaySummary>;
  readonly kind: 'native' | 'fixture' | 'preview';
  /**
   * Session metadata, stored pull-request links and the window slice of each
   * listed session's measurements, read in one snapshot. Membership is every
   * indexed session the filter admits; the window only slices measurements.
   */
  sessionsList(
    filter: SessionListFilter,
    after: string | null,
    windowDays: number,
  ): Promise<SessionPage>;
  /**
   * Exactly one session's row over the selected window, or nothing.
   *
   * Named, never searched for. The list's search matches a substring of the
   * identity and returns a bounded page of whatever it matched, so a session
   * whose identity is contained in enough others cannot be reached that way at
   * all. Same read, same rules, same snapshot as a listed row.
   */
  sessionRow(sessionId: string, windowDays: number): Promise<SessionRow | null>;
  /**
   * M-09's hands-off stretches for exactly one session over the selected
   * window: missing, unmeasured (with the excluded surface when that is why),
   * or measured in chronological order. Metadata only — no content is read, so
   * a stretch's first-call position is resolved against the transcript by the
   * caller, never by this read.
   */
  sessionStretches(sessionId: string, windowDays: number): Promise<MetricSessionStretches>;
  /**
   * One session's own transcript, read from its original local source on
   * demand and never stored. `readId` is this open's own name for the read:
   * {@link cancelSessionTranscript} abandons exactly that one, and an open
   * that is replaced or left must cancel its own rather than every read.
   *
   * What is sent is the canonical identifier and nothing else. Which file to
   * read is resolved behind this seam, from what the index recorded — a view
   * that could name a path could name any path, and this read opens files.
   */
  sessionTranscript(sessionId: string, readId: string): Promise<SessionSourceStatus>;
  /** Abandon a read. Cancelling one that has not begun is not a mistake. */
  cancelSessionTranscript(readId: string): Promise<void>;
  appInfo(): Promise<AppInfo>;
  dbCounts(): Promise<DbCounts>;
  nativeIndexStatus(): Promise<NativeIndexStatus>;
  /** Stored pull requests a session still links, with the refresh status storage holds. */
  pullRequests(): Promise<PrList>;
  /**
   * The cached per-PR linked-session report for one preset and confidence
   * mode, as the Rust core computed it. Rows overlap and are never summed; no
   * value is derived on this side. A local read: nothing is refreshed.
   */
  pullRequestAnalytics(windowDays: number, confirmedOnly: boolean): Promise<PrAnalyticsPage>;
  /**
   * One bounded page of exactly one pull request's linked sessions over the
   * report's pinned window, filtered by the store before the page bound. Linked
   * membership, not activity: a member idle in the window is listed. Metadata
   * and measurements only; no transcript is read.
   */
  pullRequestSessions(
    request: PullRequestSessionsRequest,
    after: string | null,
  ): Promise<SessionPage>;
  /**
   * Refresh the selected stored pull requests. Only storage's own identifiers
   * cross this boundary. The native side also checks pull requests on its own
   * (see {@link PrAutoCheckControls}); this call is the manual one.
   */
  refreshPullRequests(ids: number[]): Promise<PrRefreshReport>;
  /** Ask the running refresh to stop; false when none was running. */
  cancelPullRequestRefresh(): Promise<boolean>;
  /** The listener receives the event's payload as sent, unvalidated (the index status for its event). */
  subscribe(event: DataEvent, listener: (payload?: unknown) => void): Promise<Unsubscribe>;
  /** Present natively and in a fixture, which answers as its export read. */
  readonly spanDetails?: SpanDetailControls;
  /**
   * The sessions behind one day of a Dashboard report's agent time, each with
   * its time that day, the most first, capped with the rest counted — read
   * when that day's card opens. The request names the report's range, the
   * end of the window it was read over (`window.end_ms`), the zone it was
   * read in (`window.timezone`) and one of its days; the native side derives
   * the day itself in that zone, so the list adds up to that day's agent time
   * in the report even if the system zone has changed since. Metadata only.
   * Present natively and in a fixture, which answers as its export read.
   */
  daySessions?(
    windowDays: number,
    windowEndMs: number,
    timezone: string,
    date: string,
  ): Promise<DashboardDaySessions>;
  /** Present only natively, where the app checks pull requests on its own. */
  readonly prAutoCheck?: PrAutoCheckControls;
  /** Present only where a writable app database owns the policy. */
  readonly retention?: RetentionControls;
  /** Present only where a writable app database owns the setting. */
  readonly typingSpeed?: TypingSpeedControls;
  /** Present only where a writable app database owns the setting. */
  readonly humanBreak?: HumanBreakControls;
  /** Present only where a writable app database owns the setting. */
  readonly telemetry?: TelemetryControls;
  /** Present only in the native tray popover window. */
  readonly tray?: TrayControls;
  /** Present only where original local sources can be read. */
  readonly titles?: SessionTitleControls;
  readonly compactions?: SessionCompactionControls;
  /** Present natively and in a fixture, which answers as fixture mode does. */
  readonly ruleActivity?: RuleActivityControls;
  /**
   * The user's projects, read natively from the coding apps' own saved files
   * only (no Git, no conversations). Nothing is sent: the native side chooses
   * the home folder and decides every name, merge, skip, worktree fold and
   * the order. Each source's own status says whether it gave anything.
   * Present natively and in a fixture, which answers as fixture mode does.
   */
  projects?(): Promise<ProjectList>;
  /**
   * Grading runs. Present natively and in a fixture, which answers as
   * fixture mode does (`not_configured`, never a run).
   */
  readonly grading?: GradingControls;
}
