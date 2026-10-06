import type { DashboardMetrics } from './generated/DashboardMetrics';
import type { TokensByHost } from './generated/TokensByHost';
import type { EnvironmentMetrics } from './generated/EnvironmentMetrics';
import type { SessionPage } from './generated/SessionPage';
import type { SessionRow } from './generated/SessionRow';
import type { MetricSessionStretches } from './generated/MetricSessionStretches';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWebviewWindow } from '@tauri-apps/api/webviewWindow';
import type {
  DataSource,
  LiveSessionControls,
  LocalUpdateControls,
  PullRequestSessionsRequest,
  SessionListFilter,
  TrayControls,
} from './DataSource';
import type { AppInfo } from './generated/AppInfo';
import type { DbCounts } from './generated/DbCounts';
import type { NativeIndexStatus } from './generated/NativeIndexStatus';
import type { SessionSourceStatus } from './generated/SessionSourceStatus';
import type { PrList } from './generated/PrList';
import type { PrRefreshReport } from './generated/PrRefreshReport';
import type { PrAnalyticsPage } from './generated/PrAnalyticsPage';
import type { PrAutoCheckStatus } from './generated/PrAutoCheckStatus';
import { commands, trayEvents, type DataEvent } from './ipc-names';
import type { ContentRetention } from './generated/ContentRetention';
import type { ContentPurge } from './generated/ContentPurge';
import type { TodaySummary } from './generated/TodaySummary';
import type { SessionTitles } from './generated/SessionTitles';
import type { SessionCompactions } from './generated/SessionCompactions';
import type { DashboardSpanDetail } from './generated/DashboardSpanDetail';
import type { HookNames } from './generated/HookNames';
import type { RuleActivityResult } from './generated/RuleActivityResult';
import type { AccountUsage } from './generated/AccountUsage';
import type { LiveSessionSnapshot } from './generated/LiveSessionSnapshot';

/** This webview's window label; none outside a Tauri webview. */
const windowLabel = () => {
  try {
    return getCurrentWebviewWindow().label;
  } catch {
    return undefined;
  }
};
/** The window label the native tray module gives its popover. */
export const TRAY_WINDOW = 'tray';
/** Its commands refuse any other window; events reach only this one. */
export const trayControls: TrayControls = {
  visible: () => invoke<boolean>(commands.trayVisible),
  hide: () => invoke<void>(commands.trayHide),
  openMain: () => invoke<void>(commands.trayOpenMain),
  onShown: (listener) =>
    listen(trayEvents.shown, () => listener(), {
      target: { kind: 'WebviewWindow', label: TRAY_WINDOW },
    }),
  onHidden: (listener) =>
    listen(trayEvents.hidden, () => listener(), {
      target: { kind: 'WebviewWindow', label: TRAY_WINDOW },
    }),
};

export class TauriDataSource implements DataSource {
  readonly kind = 'native';
  readonly liveSessions?: LiveSessionControls;
  readonly publicUpdates?: true;
  readonly localUpdates?: LocalUpdateControls;
  constructor(enableLiveSessions = false, updatesEnabled?: boolean) {
    if (enableLiveSessions && windowLabel() === 'main') {
      if (updatesEnabled === true) this.publicUpdates = true;
      else if (updatesEnabled === false)
        this.localUpdates = {
          viewPublicReleases: () => invoke<void>(commands.openPublicReleases),
        };
    }
    if (enableLiveSessions) {
      this.liveSessions = {
        read: (sessionIds, viewId) =>
          invoke<LiveSessionSnapshot>(commands.liveSessionsRead, { sessionIds, viewId }),
        release: (viewId) => invoke<void>(commands.liveSessionsRelease, { viewId }),
      };
    }
  }
  accountUsage() {
    return invoke<AccountUsage>(commands.accountUsage);
  }
  refreshClaudeUsage() {
    return invoke<AccountUsage>(commands.refreshClaudeUsage);
  }
  readonly tray = windowLabel() === TRAY_WINDOW ? trayControls : undefined;
  readonly prAutoCheck = {
    status: () => invoke<PrAutoCheckStatus>(commands.prAutoCheckStatus),
    request: () => invoke<PrAutoCheckStatus>(commands.prAutoCheck),
  };
  readonly retention = {
    read: () => invoke<ContentRetention>(commands.contentRetention),
    set: (mode: ContentRetention) =>
      invoke<ContentRetention>(commands.setContentRetention, { mode }),
    purge: () => invoke<ContentPurge>(commands.purgeStoredContent),
  };
  readonly titles = {
    read: (sessionIds: readonly string[], readId: string) =>
      invoke<SessionTitles>(commands.sessionTitles, { sessionIds, readId }),
    cancel: (readId: string) => invoke<void>(commands.cancelSessionTitles, { readId }),
  };
  readonly compactions = {
    read: (sessionIds: readonly string[], readId: string) =>
      invoke<SessionCompactions>(commands.sessionCompactions, { sessionIds, readId }),
    cancel: (readId: string) => invoke<void>(commands.cancelSessionCompactions, { readId }),
  };
  readonly spanDetails = {
    read: (sessionId: string, startMs: number, endMs: number, readId: string) =>
      invoke<DashboardSpanDetail>(commands.spanDetail, { sessionId, startMs, endMs, readId }),
    cancel: (readId: string) => invoke<void>(commands.cancelSpanDetail, { readId }),
  };
  readonly hookNames = {
    read: (windowDays: number, windowEndMs: number, readId: string) =>
      invoke<HookNames>(commands.environmentHookNames, { windowDays, windowEndMs, readId }),
    cancel: (readId: string) => invoke<void>(commands.cancelEnvironmentHookNames, { readId }),
  };
  readonly ruleActivity = {
    read: (readId: string) => invoke<RuleActivityResult>(commands.ruleActivityRead, { readId }),
    cancel: (readId: string) => invoke<void>(commands.ruleActivityCancel, { readId }),
  };
  readonly typingSpeed = {
    read: () => invoke<number>(commands.typingSpeed),
    set: (wpm: number) => invoke<number>(commands.setTypingSpeed, { wpm }),
    openTest: () => invoke<void>(commands.openTypingTest),
  };
  readonly humanBreak = {
    read: () => invoke<number>(commands.humanBreak),
    set: (minutes: number) => invoke<number>(commands.setHumanBreak, { minutes }),
  };
  dashboard(windowDays: number) {
    return invoke<DashboardMetrics>(commands.dashboard, { windowDays });
  }
  today() {
    return invoke<TodaySummary>(commands.today);
  }

  tokensByHost(windowDays: number) {
    return invoke<TokensByHost>(commands.tokensByHost, { windowDays });
  }
  environment(windowDays: number) {
    return invoke<EnvironmentMetrics>(commands.environment, { windowDays });
  }
  sessionsList(
    { search, hosts, withPrs }: SessionListFilter,
    after: string | null,
    windowDays: number,
  ) {
    return invoke<SessionPage>(commands.sessionsList, {
      search,
      hosts,
      withPrs,
      after,
      windowDays,
    });
  }
  sessionRow(sessionId: string, windowDays: number) {
    return invoke<SessionRow | null>(commands.sessionRow, { sessionId, windowDays });
  }
  sessionStretches(sessionId: string, windowDays: number) {
    return invoke<MetricSessionStretches>(commands.sessionStretches, { sessionId, windowDays });
  }
  sessionTranscript(sessionId: string, readId: string) {
    return invoke<SessionSourceStatus>(commands.sessionTranscript, { sessionId, readId });
  }
  cancelSessionTranscript(readId: string) {
    return invoke<void>(commands.cancelSessionTranscript, { readId });
  }
  appInfo() {
    return invoke<AppInfo>(commands.appInfo);
  }
  dbCounts() {
    return invoke<DbCounts>(commands.dbCounts);
  }
  nativeIndexStatus() {
    return invoke<NativeIndexStatus>(commands.nativeIndexStatus);
  }
  pullRequests() {
    return invoke<PrList>(commands.pullRequests);
  }
  pullRequestAnalytics(windowDays: number, confirmedOnly: boolean) {
    return invoke<PrAnalyticsPage>(commands.pullRequestAnalytics, { windowDays, confirmedOnly });
  }
  pullRequestSessions(
    { repository, number, confirmedOnly, windowDays, windowEndMs }: PullRequestSessionsRequest,
    after: string | null,
  ) {
    return invoke<SessionPage>(commands.pullRequestSessions, {
      repository,
      number,
      confirmedOnly,
      windowDays,
      windowEndMs,
      after,
    });
  }
  refreshPullRequests(ids: number[]) {
    return invoke<PrRefreshReport>(commands.refreshPullRequests, { ids });
  }
  cancelPullRequestRefresh() {
    return invoke<boolean>(commands.cancelPullRequestRefresh);
  }
  subscribe(event: DataEvent, listener: (payload?: unknown) => void) {
    return listen(event, ({ payload }) => listener(payload));
  }
}
