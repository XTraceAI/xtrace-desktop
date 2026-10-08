import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import type { DashboardUsageGate } from '../data/generated/DashboardUsageGate';
import type { DashboardWindow } from '../data/generated/DashboardWindow';
import type { MetricPrFreshness } from '../data/generated/MetricPrFreshness';
import type { MetricPrMarker } from '../data/generated/MetricPrMarker';
import type { PrAttemptOutcome } from '../data/generated/PrAttemptOutcome';
import type { PrRefreshStatusReport } from '../data/generated/PrRefreshStatusReport';
import type { PrRow } from '../data/generated/PrRow';
import type { TodaySummary } from '../data/generated/TodaySummary';
import { clock } from '../kit/clock';
import { usd as kitUsd } from '../kit/format';
import { HostGlyph } from '../kit/HostGlyph';
import { HOST_NAMES, hostName, surfaceLabel as kitSurfaceLabel } from '../kit/hosts';
import { agentDuration, agentTime } from './agent-duration';
import { markerText } from './dashboard/effort-chart';
import { spanDuration } from './dashboard/LaneSpan';
import {
  outcomeText,
  refreshStateLabel,
  refreshStatusText,
  refreshStatusWords,
  rowStatusText,
} from './dashboard/pr-effort';
import { clockTime, recordedTime, surfaceLabel, usd } from './dashboard/present';
import { LIVE_SESSION_HOSTS } from './live-session-status';
import {
  evidenceWords,
  freshnessText,
  gatePercentText,
  gateText,
  gateVerdict,
} from './pr-analytics';
import { evidenceWords as cellEvidenceWords, HandsOff, handsOffReason } from './session-cells';
import { handsOffSpoken, handsOffTime } from './metric-format';
import { activeDuration, stretchDuration, unmeasuredText } from './session-detail/SessionTimeline';
import { agentText } from './tray/TrayPage';
import { hostName as welcomeHostName } from './welcome/welcome-text';
import type { SessionRow } from '../data/generated/SessionRow';

/**
 * One question, one answer: each value below is written by one function, and
 * every screen that shows it gets the same text. Changing the one function
 * changes them all together.
 */

afterEach(cleanup);

const window: DashboardWindow = {
  days: 7,
  start_ms: Date.parse('2026-09-29T00:00:00Z'),
  end_ms: Date.parse('2026-10-06T00:00:00Z'),
  timezone: 'UTC',
  clock: 'fixture',
};

it('writes one agent time the same way on every screen, and no non-time as one', () => {
  const ms = 11_568_000;
  const shown = agentTime(ms);
  expect(shown).toBe('3 h 13 m');
  expect(agentDuration(ms).visible).toBe(shown);
  // A lane's active span, a hands-off stretch and its active part, the tray.
  expect(spanDuration(0, ms)).toBe(shown);
  expect(activeDuration(ms)).toBe(shown);
  const today = { agent: { active_ms: ms, sessions: 2 } } as TodaySummary;
  expect(agentText(today).value).toBe(shown);
  // Read aloud, the same value in words.
  expect(activeDuration(ms, true)).toBe(agentDuration(ms).spoken);
  expect(spanDuration(0, ms, true)).toBe(agentDuration(ms).spoken);
  // Not a duration: unknown, never `—hNaNm` or `-1h-1m`.
  for (const bad of [Number.NaN, Number.POSITIVE_INFINITY, -60_000])
    expect(agentDuration(bad)).toEqual({ visible: '—', spoken: 'not measured', exact: '—' });
  // A measured zero is a zero, never a dash.
  expect(agentTime(0)).toBe('0 h 0 m');
});

it('writes money one way: cents below $100, whole dollars from $100', () => {
  expect(usd).toBe(kitUsd);
  expect(usd(250.4)).toBe('$250');
  expect(usd(25.04)).toBe('$25.04');
});

it('writes a report instant with the one clock, in the report zone', () => {
  const ms = Date.parse('2026-10-05T14:30:00Z');
  expect(clockTime(ms, window)).toBe(clock(ms, { date: 'day', timeZone: 'UTC' }));
  expect(clockTime(ms, window, false)).toBe(clock(ms, { timeZone: 'UTC' }));
  expect(recordedTime(ms, window)).toBe(`${clock(ms, { date: 'year', timeZone: 'UTC' })} (UTC)`);
});

const statuses: [PrRefreshStatusReport, MetricPrFreshness][] = [
  [{ status: 'never_attempted' }, { state: 'never_attempted' }],
  [{ status: 'refreshed' }, { state: 'refreshed' }],
  [
    { status: 'failed_never_refreshed', error: 'timeout' },
    { state: 'failed_never_refreshed', error: 'timeout' },
  ],
  [
    { status: 'failed_after_refresh', error: 'rate_limited' },
    { state: 'failed_after_refresh', error: 'rate_limited' },
  ],
];

it('words a pull request’s refresh status one way in the refresh dialog, chart and PRs page', () => {
  for (const [status, freshness] of statuses) {
    const words = refreshStatusText(status);
    // The stored row and the report's freshness are the same status in the same words.
    expect(refreshStatusText(freshness)).toBe(words);
    expect(refreshStatusWords(status)).toEqual(refreshStatusWords(freshness));
    const row = { status, refreshed_at_ms: null } as unknown as PrRow;
    expect(rowStatusText(row, window).startsWith(words)).toBe(true);
    expect(freshnessText(freshness).startsWith(words)).toBe(true);
    const marker = {
      repository: 'example/app',
      number: 1,
      work_type: 'feat',
      confidence: 'exact',
      freshness,
    } as MetricPrMarker;
    expect(markerText(marker).endsWith(words)).toBe(true);
  }
  expect(refreshStatusText(statuses[2]![0])).toBe('could not be checked (timed out)');
  // The Merged PRs tile counts pull requests with the very same words.
  expect(refreshStateLabel.never_attempted).toBe('not checked yet');
});

it('names a link’s evidence with one set of words, never the stored `sha`', () => {
  expect(cellEvidenceWords).toBe(evidenceWords);
  const marker = {
    repository: 'example/app',
    number: 4,
    work_type: 'feat',
    confidence: 'sha',
    freshness: { state: 'refreshed' },
  } as MetricPrMarker;
  expect(markerText(marker)).toContain(`· ${evidenceWords.sha} link ·`);
  expect(markerText(marker)).not.toContain('sha');
});

it('names a host and an unlabelled surface one way everywhere', () => {
  expect(welcomeHostName).toBe(hostName);
  expect(surfaceLabel).toBe(kitSurfaceLabel);
  expect(LIVE_SESSION_HOSTS.claude.label).toBe(HOST_NAMES.claude);
  expect(LIVE_SESSION_HOSTS.codex.label).toBe(HOST_NAMES.codex);
  render(<HostGlyph host="claude" />);
  expect(screen.getByRole('img', { name: hostName('claude') })).toBeTruthy();
  expect(hostName('claude')).toBe('Claude Code');
  expect(hostName('legacy')).toBe('legacy');
  const unlabelled = surfaceLabel('codex', null);
  expect(unlabelled).toBe('Codex · unknown surface');
  const excluded = {
    host: 'codex',
    surface: null,
    qualifying_sessions: 3,
    degenerate_sessions: 2,
  };
  expect(unmeasuredText(excluded)).toContain(unlabelled);
  const row = {
    hands_off: { state: 'unmeasured', excluded_surface: excluded },
  } as unknown as SessionRow;
  expect(handsOffReason(row)).toContain(unlabelled);
});

it('never shows a failing coverage gate at a percent that reads as passing', () => {
  const gate: DashboardUsageGate = {
    window_start_ms: window.start_ms,
    window_end_ms: window.end_ms,
    eligible_sessions: 10_000,
    measured_sessions: 8_999,
    pct: 89.99,
    passes: false,
    excluded_surfaces: [],
  };
  expect(gatePercentText(89.99)).toBe('89.9%');
  expect(gatePercentText(90)).toBe('90%');
  expect(gatePercentText(100)).toBe('100%');
  expect(gateText(gate, window)).toContain(`(89.9%, ${gateVerdict(false)})`);
  expect(gateVerdict(true)).toBe('meets the 90% gate');
});

it('writes hands-off time one way, as minutes, for a stretch and for a median', () => {
  // A three-minute stretch on the timeline and a three-minute median read alike.
  expect(stretchDuration(180_000)).toBe(handsOffTime(3));
  expect(handsOffTime(3)).toBe('3 min');
  expect(stretchDuration(180_000, true)).toBe(handsOffSpoken(3));
  const row = {
    hands_off: { state: 'measured', median_min: 3, n: 2 },
  } as unknown as SessionRow;
  render(<HandsOff row={row} />);
  expect(screen.getByText(handsOffTime(3))).toBeTruthy();
  expect(handsOffTime(Number.NaN)).toBe('—');
});

it('lets a glyph take the name the text beside it uses', () => {
  render(<HostGlyph host="claude" label="Claude" />);
  expect(screen.getByRole('img', { name: 'Claude' }).getAttribute('title')).toBe('Claude');
});

it('words an attempt’s result with the same words as the status it leaves', () => {
  const checked = outcomeText({
    outcome: 'succeeded',
    persistence: { persistence: 'recorded', write: 'applied' },
  } as PrAttemptOutcome);
  expect(checked.toLowerCase().startsWith(refreshStateLabel.refreshed)).toBe(true);
  const failed = outcomeText({
    outcome: 'failed',
    error: 'timeout',
    persistence: { persistence: 'recorded', write: 'unchanged' },
  } as PrAttemptOutcome);
  expect(failed.toLowerCase().startsWith(refreshStateLabel.failed_never_refreshed)).toBe(true);
});

it('words a not-found answer the same way in the row, the result and the inventory', () => {
  const label = refreshStateLabel.not_found_on_github;
  const row = {
    status: { status: 'not_found_on_github' },
    refreshed_at_ms: null,
    last_attempted_at_ms: null,
  } as unknown as PrRow;
  expect(refreshStatusWords(row.status).label).toBe(label);
  expect(rowStatusText(row, window).startsWith(label)).toBe(true);
  const result = outcomeText(
    {
      outcome: 'failed',
      error: 'not_found',
      persistence: { persistence: 'recorded', write: 'applied' },
    } as PrAttemptOutcome,
    row,
  );
  expect(result.toLowerCase().startsWith(label.toLowerCase())).toBe(true);
  // Facts kept from an earlier check are stale for a reason that is not a failure.
  expect(refreshStatusText({ state: 'failed_after_refresh', error: 'not_found' })).toBe(
    refreshStateLabel.not_found_after_refresh,
  );
});
