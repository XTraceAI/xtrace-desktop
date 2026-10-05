import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AccountUsage } from '../data/generated/AccountUsage';
import type { AccountUsageWindow } from '../data/generated/AccountUsageWindow';
import {
  AccountUsageWidget,
  ageLabel,
  limitingWindow,
  dayValueLabel,
  paceGapLabel,
  readingsSpanVisible,
  remainingLabel,
  resetLabel,
  shortTime,
  weekDays,
} from './AccountUsageWidget';

afterEach(() => {
  cleanup();
  // Restore spies first: a spy on a faked timer would otherwise put the fake
  // back after the real clock returns, and later tooltips would never open.
  vi.restoreAllMocks();
  vi.useRealTimers();
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

it('rounds a provider reset just before 8 PM and one exactly at 8 PM to the same local minute', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  expect(Intl.DateTimeFormat().resolvedOptions().timeZone).toBe('America/Los_Angeles');
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-09-24T18:00:00Z'));
  const justBefore = new Date('2026-09-29T02:59:59.968Z').getTime() / 1000;
  const exactMinute = new Date('2026-09-29T03:00:00.249Z').getTime() / 1000;
  expect(resetLabel(justBefore)).toBe('resets Mon 8 PM');
  expect(resetLabel(exactMinute)).toBe('resets Mon 8 PM');
  expect(resetLabel(null)).toBe('reset time unknown');
});

it('writes short local times without a time-zone name', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  const now = new Date('2026-10-01T19:00:00Z').getTime(); // Thu noon PDT
  const at = (iso: string) => new Date(iso).getTime() / 1000;
  expect(shortTime(at('2026-10-01T20:10:00Z'), now)).toBe('today 1:10 PM');
  expect(shortTime(at('2026-10-06T03:00:00Z'), now)).toBe('Mon 8 PM');
  expect(shortTime(at('2026-10-07T21:30:00Z'), now)).toBe('Wed 2:30 PM');
  expect(shortTime(at('2026-10-12T19:00:00Z'), now)).toBe('Oct 12');
  expect(shortTime(Number.NaN, now)).toBeNull();
});

it('keeps whole times on a 24-hour clock', () => {
  vi.stubEnv('TZ', 'Europe/London');
  // The widget formats in the default locale; make that British English.
  class BritishFormat extends Intl.DateTimeFormat {
    constructor(_locales?: Intl.LocalesArgument, options?: Intl.DateTimeFormatOptions) {
      super('en-GB', options);
    }
  }
  vi.stubGlobal('Intl', Object.assign(Object.create(Intl), { DateTimeFormat: BritishFormat }));
  const now = new Date('2026-10-01T09:00:00Z').getTime(); // Thu 10:00 BST
  const at = (iso: string) => new Date(iso).getTime() / 1000;
  expect(shortTime(at('2026-10-01T19:00:00Z'), now)).toBe('today 20:00');
  expect(shortTime(at('2026-10-05T19:30:00Z'), now)).toBe('Mon 20:30');
});

it('shows the read age only once it is five minutes old or stale', () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-10-01T19:00:00Z'));
  const now = Date.now() / 1000;
  expect(ageLabel(now - 60, false)).toBeNull();
  expect(ageLabel(now - 60, true)).toBe('1 min ago');
  expect(ageLabel(now - 10, true)).toBe('just now');
  expect(ageLabel(now - 12 * 60, false)).toBe('12 min ago');
  expect(ageLabel(now - 3 * 3600, false)).toBe('3 hr ago');
  expect(ageLabel(now - 2 * 86_400, true)).toBe('2 days ago');
  expect(ageLabel(now + 600, false)).toBe('read time uncertain');
});

const usage: AccountUsage = {
  claude: {
    state: 'available',
    issue: null,
    checked_at: 1790618400,
    windows: [
      {
        bucket_key: 'claude',
        window_key: 'five_hour',
        scope: 'all_models',
        name: 'Claude',
        window: 'Session',
        used_percent: 0,
        duration_minutes: 300,
        resets_at: null,
      },
      {
        bucket_key: 'claude',
        window_key: 'seven_day',
        scope: 'all_models',
        name: 'Claude',
        window: 'Weekly',
        used_percent: 100,
        duration_minutes: 10080,
        resets_at: Math.floor(Date.now() / 1000) + 3600,
      },
      {
        bucket_key: 'claude:example',
        window_key: 'seven_day',
        scope: 'model',
        name: 'Example model',
        window: 'Weekly',
        used_percent: 68,
        duration_minutes: 10080,
        resets_at: null,
      },
    ],
  },
  codex: {
    state: 'available',
    issue: null,
    checked_at: 1790618400,
    windows: [
      {
        bucket_key: 'codex',
        window_key: 'primary',
        scope: 'all_models',
        name: 'Codex',
        window: 'Weekly',
        used_percent: 26,
        duration_minutes: 10080,
        resets_at: 1791050717,
      },
    ],
  },
};

it('puts the exhausted all-model week above an unused session and keeps model limits separate', () => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date('2026-10-01T19:00:00Z'));
  render(
    <AccountUsageWidget usage={usage} failed={false} refreshing={false} onRefresh={vi.fn()} />,
  );
  const claude = screen.getByLabelText(
    'Claude account usage: 0% remaining · Weekly · limit reached',
  );
  expect(claude.textContent).toContain('Limit reached');
  expect(
    (claude.querySelector('.xt-account-summary-track > span') as HTMLElement).style.width,
  ).toBe('0%');
  expect(claude.textContent).toContain('resets');
  expect(claude.textContent).toContain('days ago');
  expect(claude.textContent).not.toContain('Last read');
  const codex = screen.getByLabelText('Codex account usage: 74% remaining · Weekly');
  expect(codex.textContent).toContain('Weekly');
  expect((codex.querySelector('.xt-account-summary-track > span') as HTMLElement).style.width).toBe(
    '74%',
  );
  fireEvent.click(claude);
  // Claude's details do not list its session or model-specific limits.
  const claudeDetail = claude.closest('details')!.querySelector<HTMLElement>('.xt-account-detail')!;
  expect(within(claudeDetail).queryByText('Session')).toBeNull();
  expect(within(claudeDetail).queryByText('Example model · Weekly')).toBeNull();
  expect(claudeDetail.querySelectorAll('.xt-account-window-head')).toHaveLength(0);
  cleanup();
  // Codex lists the same kinds of windows as rows.
  render(
    <AccountUsageWidget
      usage={{
        ...usage,
        codex: { ...usage.claude, windows: usage.claude.windows.map(toCodex) },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  const codexSummary = screen.getByLabelText(
    'Codex account usage: 0% remaining · Weekly · limit reached',
  );
  fireEvent.click(codexSummary);
  const detail = codexSummary.closest('details')!;
  expect(within(detail).getByText('Session')).toBeTruthy();
  expect(within(detail).getByText('Example model · Weekly')).toBeTruthy();
  expect(within(detail).getByText('100% remaining')).toBeTruthy();
  expect(within(detail).getByText('32% remaining')).toBeTruthy();
  const session = within(detail).getByText('Session').closest('.xt-account-window')!;
  expect((session.querySelector('.xt-account-track > span') as HTMLElement).style.width).toBe(
    '100%',
  );
  const model = within(detail).getByText('Example model · Weekly').closest('.xt-account-window')!;
  expect((model.querySelector('.xt-account-track > span') as HTMLElement).style.width).toBe('32%');
});

/** The same window as Codex reports it. */
function toCodex(window: AccountUsageWindow): AccountUsageWindow {
  return { ...window, bucket_key: window.bucket_key.replace(/^claude/, 'codex') };
}

it('shows missing and failed providers without guessed percentages or cached values', () => {
  const { rerender } = render(
    <AccountUsageWidget
      usage={{
        claude: {
          state: 'unavailable',
          issue: 'credential_access_required',
          checked_at: null,
          windows: [],
        },
        codex: { state: 'available', issue: null, checked_at: 1790618400, windows: [] },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  expect(screen.getByLabelText('Claude account usage: Usage unavailable').textContent).toContain(
    'Account access required',
  );
  expect(
    screen
      .getByLabelText('Claude account usage: Usage unavailable')
      .querySelector('.xt-account-summary-track'),
  ).toBeNull();
  expect(
    screen.getByLabelText('Codex account usage: No limits reported').textContent,
  ).not.toContain('% remaining');
  expect(
    screen
      .getByLabelText('Codex account usage: No limits reported')
      .querySelector('.xt-account-summary-track'),
  ).toBeNull();
  rerender(
    <AccountUsageWidget
      usage={{
        ...usage,
        claude: {
          state: 'stale',
          issue: 'rate_limited',
          checked_at: 1790618400,
          windows: usage.claude.windows,
        },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  const stale = screen.getByLabelText(
    'Claude account usage: 0% remaining · Weekly · limit reached · stale',
  );
  expect(stale.textContent).toContain('Stale');
  expect(stale.textContent).toContain('days ago');
  rerender(
    <AccountUsageWidget
      usage={{
        ...usage,
        claude: {
          ...usage.claude,
          windows: usage.claude.windows.map((window) =>
            window.window_key === 'seven_day' && window.scope === 'all_models'
              ? { ...window, resets_at: Math.floor(Date.now() / 1000) - 60 }
              : window,
          ),
        },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  expect(
    screen.getByLabelText('Claude account usage: Reset passed · Refresh for current limits')
      .textContent,
  ).not.toContain('% remaining');
  expect(
    screen
      .getByLabelText('Claude account usage: Reset passed · Refresh for current limits')
      .querySelector('.xt-account-summary-track'),
  ).toBeNull();
  rerender(<AccountUsageWidget usage={usage} failed refreshing={false} onRefresh={vi.fn()} />);
  expect(screen.queryByText('0% remaining')).toBeNull();
  expect(
    screen
      .queryByRole('region', { name: 'Account usage' })
      ?.querySelector('.xt-account-summary-track'),
  ).toBeNull();
  expect(screen.getByText(/could not be read/)).toBeTruthy();
  rerender(<AccountUsageWidget failed={false} refreshing={false} onRefresh={vi.fn()} />);
  expect(
    screen
      .getByLabelText('Claude account usage: Reading…')
      .querySelector('.xt-account-summary-track'),
  ).toBeNull();
});

it('asks for a refresh before the first Claude read and keeps a real missing source an error', () => {
  const empty = { state: 'unavailable', checked_at: null } as const;
  const { rerender } = render(
    <AccountUsageWidget
      usage={{ claude: { ...empty, issue: 'not_fetched', windows: [] }, codex: usage.codex }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  const claude = screen.getByLabelText('Claude account usage: Refresh to load usage');
  expect(claude.textContent).toContain('Refresh to load usage');
  expect(claude.textContent).not.toContain('unavailable');
  expect(claude.querySelector('.xt-account-summary-track')).toBeNull();
  expect(claude.closest('details')!.querySelector('.xt-account-detail')!.textContent).toBe(
    'Refresh to load usage',
  );
  rerender(
    <AccountUsageWidget
      usage={{ claude: { ...empty, issue: 'source_unavailable', windows: [] }, codex: usage.codex }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  const missing = screen.getByLabelText('Claude account usage: Usage unavailable');
  expect(missing.textContent).toContain('Usage source unavailable');
  expect(screen.queryByText('Refresh to load usage')).toBeNull();
});

it('shows Reading while the first automatic Claude read runs, and keeps a saved reading stale', () => {
  const { rerender } = render(
    <AccountUsageWidget
      usage={{
        claude: { state: 'unavailable', issue: 'reading', checked_at: null, windows: [] },
        codex: usage.codex,
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  const claude = screen.getByLabelText('Claude account usage: Reading…');
  expect(claude.textContent).toContain('Reading…');
  expect(claude.textContent).not.toContain('Refresh to load usage');
  expect(claude.textContent).not.toContain('unavailable');
  expect(claude.querySelector('.xt-account-summary-track')).toBeNull();
  // A reading saved by an earlier run stays visible, marked stale with its age.
  rerender(
    <AccountUsageWidget
      usage={{
        ...usage,
        claude: { ...usage.claude, state: 'stale', issue: 'reading' },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  const stale = screen.getByLabelText(
    'Claude account usage: 0% remaining · Weekly · limit reached · stale',
  );
  expect(stale.textContent).toContain('Stale');
  expect(stale.textContent).toContain('days ago');
  expect(stale.querySelector('.xt-account-summary-track')).not.toBeNull();
});

it('labels remaining capacity without rounding a small positive balance to zero', () => {
  expect(remainingLabel(0)).toBe('100% remaining');
  expect(remainingLabel(35)).toBe('65% remaining');
  expect(remainingLabel(67.9)).toBe('32.1% remaining');
  expect(remainingLabel(100)).toBe('0% remaining');
  expect(remainingLabel(99.9)).toBe('0.1% remaining');
  expect(remainingLabel(99.99)).toBe('<0.1% remaining');
  expect(remainingLabel(0.01)).toBe('99.9% remaining');
  expect(limitingWindow(usage.claude.windows)?.window_key).toBe('seven_day');
});

it('keeps a valid weekly limit when separate model and session windows have reset', () => {
  const expired = Math.floor(Date.now() / 1000) - 60;
  const claude = {
    ...usage.claude,
    windows: usage.claude.windows.map((window) =>
      window.window_key === 'seven_day' && window.scope === 'all_models'
        ? window
        : { ...window, resets_at: expired },
    ),
  };
  render(
    <AccountUsageWidget
      usage={{ ...usage, claude }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  const summary = screen.getByLabelText(
    'Claude account usage: 0% remaining · Weekly · limit reached',
  );
  expect(summary.textContent).toContain('0% remaining');
  fireEvent.click(summary);
  expect(summary.closest('details')!.querySelector('.xt-account-window-head')).toBeNull();
  cleanup();
  render(
    <AccountUsageWidget
      usage={{ ...usage, codex: { ...claude, windows: claude.windows.map(toCodex) } }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  fireEvent.click(
    screen.getByLabelText('Codex account usage: 0% remaining · Weekly · limit reached'),
  );
  const model = screen.getByText('Example model · Weekly').closest('.xt-account-window')!;
  expect(model.textContent).toContain('Reset passed · Refresh');
  expect(model.textContent).not.toContain('% remaining');
  const session = screen.getByText('Session').closest('.xt-account-window')!;
  expect(session.textContent).toContain('Reset passed · Refresh');
  expect(session.textContent).not.toContain('% remaining');
});

it('hides an expired main limit at its reset without a parent render or provider read', async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-09-28T18:00:00Z'));
  const reset = Math.floor(Date.now() / 1000) + 2;
  const onRefresh = vi.fn();
  const claude = {
    ...usage.claude,
    windows: usage.claude.windows.map((window) =>
      window.window_key === 'seven_day' && window.scope === 'all_models'
        ? { ...window, resets_at: reset }
        : window,
    ),
  };
  render(
    <AccountUsageWidget
      usage={{ ...usage, claude }}
      failed={false}
      refreshing={false}
      onRefresh={onRefresh}
    />,
  );
  expect(
    screen.getByLabelText('Claude account usage: 0% remaining · Weekly · limit reached'),
  ).toBeTruthy();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2000);
  });
  expect(
    screen.getByLabelText('Claude account usage: Reset passed · Refresh for current limits')
      .textContent,
  ).not.toContain('% remaining');
  expect(onRefresh).not.toHaveBeenCalled();
});

it('keeps a valid main week visible when only a model and session reset', async () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-09-28T18:00:00Z'));
  const reset = Math.floor(Date.now() / 1000) + 2;
  const claude = {
    ...usage.claude,
    windows: usage.claude.windows.map((window) =>
      window.window_key === 'seven_day' && window.scope === 'all_models'
        ? { ...window, resets_at: reset + 3600 }
        : { ...window, resets_at: reset },
    ),
  };
  render(
    <AccountUsageWidget
      usage={{ ...usage, claude }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  const summary = screen.getByLabelText(
    'Claude account usage: 0% remaining · Weekly · limit reached',
  );
  fireEvent.click(summary);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2000);
  });
  expect(
    screen.getByLabelText('Claude account usage: 0% remaining · Weekly · limit reached'),
  ).toBeTruthy();
  // Claude's details list no other windows, before or after their reset.
  expect(screen.queryByText('Example model · Weekly')).toBeNull();
  expect(screen.queryByText('Session')).toBeNull();
});

it('bounds a distant reset timer and ignores an invalid reset time', () => {
  vi.useFakeTimers();
  vi.setSystemTime(new Date('2026-09-28T18:00:00Z'));
  const timer = vi.spyOn(window, 'setTimeout');
  const week = usage.claude.windows[1];
  const distant = { ...week, resets_at: Math.floor(Date.now() / 1000) + 3_000_000 };
  const { rerender } = render(
    <AccountUsageWidget
      usage={{
        claude: { ...usage.claude, windows: [distant] },
        codex: { ...usage.codex, windows: [] },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  expect(timer).toHaveBeenCalledWith(expect.any(Function), 2_147_483_647);
  rerender(
    <AccountUsageWidget
      usage={{
        claude: { ...usage.claude, windows: [{ ...week, resets_at: Number.MAX_SAFE_INTEGER }] },
        codex: { ...usage.codex, windows: [] },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  expect(vi.getTimerCount()).toBe(0);
});

it('keeps a provider-named bucket whose model scope is unspecified', () => {
  const bucket = {
    bucket_key: 'codex:example',
    window_key: 'primary',
    scope: 'unspecified' as const,
    name: 'Example bucket',
    window: 'Weekly',
    used_percent: 42,
    duration_minutes: 10080,
    resets_at: null,
  };
  render(
    <AccountUsageWidget
      usage={{
        ...usage,
        codex: { state: 'available', issue: null, checked_at: 1790618400, windows: [bucket] },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  expect(
    screen.getByLabelText('Codex account usage: 58% remaining · Example bucket · Weekly'),
  ).toBeTruthy();
});

it('keeps Refresh keyboard reachable and disables duplicate clicks during a read', () => {
  const refresh = vi.fn();
  const { rerender } = render(
    <AccountUsageWidget usage={usage} failed={false} refreshing={false} onRefresh={refresh} />,
  );
  const button = screen.getByRole('button', { name: 'Refresh usage' });
  // An icon only: the name comes from the label, and the drawing is hidden.
  expect(button.textContent).toBe('');
  expect(button.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
  // WebKit (the app's web view) reaches a button by Tab only with an explicit tabindex.
  expect(button.getAttribute('tabindex')).toBe('0');
  button.focus();
  expect(document.activeElement).toBe(button);
  fireEvent.click(button);
  expect(refresh).toHaveBeenCalledOnce();
  rerender(<AccountUsageWidget usage={usage} failed={false} refreshing onRefresh={refresh} />);
  expect(button.getAttribute('aria-label')).toBe('Reading usage…');
  expect(button.hasAttribute('data-reading')).toBe(true);
  expect((button as HTMLButtonElement).disabled).toBe(true);
  fireEvent.click(button);
  expect(refresh).toHaveBeenCalledOnce();
  rerender(
    <AccountUsageWidget failed refreshing={false} refreshEnabled={false} onRefresh={refresh} />,
  );
  expect(button.getAttribute('aria-label')).toBe('Refresh usage');
  expect(button.hasAttribute('data-reading')).toBe(false);
  expect((button as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByText('Account usage is available in the desktop app.')).toBeTruthy();
});

it('names Refresh in a tooltip on hover and on focus', async () => {
  render(
    <AccountUsageWidget usage={usage} failed={false} refreshing={false} onRefresh={vi.fn()} />,
  );
  const button = screen.getByRole('button', { name: 'Refresh usage' });
  fireEvent.mouseEnter(button);
  fireEvent.mouseMove(button);
  expect((await screen.findByRole('tooltip')).textContent).toBe('Refresh usage');
  cleanup();
  render(
    <AccountUsageWidget usage={usage} failed={false} refreshing={false} onRefresh={vi.fn()} />,
  );
  fireEvent.focus(screen.getByRole('button', { name: 'Refresh usage' }));
  expect((await screen.findByRole('tooltip')).textContent).toBe('Refresh usage');
});

const forecastText = [
  'How the forecast works',
  'Usage is read every 10 minutes for Claude, and every 5 minutes for Codex while this window is open. Readings stay on this Mac.',
  "Your speed is how fast usage rose over the last 24 hours. Until there's enough history, it's your average since the week started.",
  "That speed is extended to the reset. If it reaches 100% first, you see when you'd run out.",
  "The tick on the bar marks where you'd be with even use through the week.",
  "It assumes a steady pace, so nights and weekends aren't taken into account.",
];

function forecastNote() {
  return screen.queryAllByRole('tooltip').find((tip) => tip.querySelector('ul')) ?? null;
}

function expectForecastNote(note: HTMLElement | null) {
  expect(note).not.toBeNull();
  expect(note!.querySelector('strong')?.textContent).toBe(forecastText[0]);
  expect(Array.from(note!.querySelectorAll('li'), (item) => item.textContent)).toEqual(
    forecastText.slice(1),
  );
}

describe('how the forecast works', () => {
  const renderWidget = () =>
    render(
      <AccountUsageWidget usage={usage} failed={false} refreshing={false} onRefresh={vi.fn()} />,
    );
  const info = () => screen.getByRole('button', { name: 'How the forecast works' });

  it('sits left of Refresh, before the rows in Tab order', () => {
    renderWidget();
    const order = Array.from(
      document.querySelectorAll<HTMLElement>('button, summary'),
      (element) => element.getAttribute('aria-label') ?? '',
    );
    expect(order.slice(0, 4)).toEqual([
      'How the forecast works',
      'Refresh usage',
      expect.stringMatching(/^Claude account usage/),
      expect.stringMatching(/^Codex account usage/),
    ]);
    expect(info().getAttribute('tabindex')).toBe('0');
    expect(info().textContent).toBe('');
    expect(forecastNote()).toBeNull();
  });

  it('opens on hover and closes when the pointer leaves', async () => {
    renderWidget();
    fireEvent.mouseEnter(info());
    fireEvent.mouseMove(info());
    await waitFor(() => expectForecastNote(forecastNote()));
    expect(info().getAttribute('aria-describedby')).toBe(forecastNote()!.id);
    fireEvent.mouseLeave(info());
    await waitFor(() => expect(forecastNote()).toBeNull());
  });

  it('opens on keyboard focus and closes on Escape', async () => {
    renderWidget();
    fireEvent.focus(info());
    await waitFor(() => expectForecastNote(forecastNote()));
    fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Escape' });
    await waitFor(() => expect(forecastNote()).toBeNull());
    expect(info().getAttribute('aria-expanded')).toBe('false');
  });

  it('opens on click, stays open when the pointer leaves, and closes on a second click', async () => {
    renderWidget();
    fireEvent.click(info());
    await waitFor(() => expectForecastNote(forecastNote()));
    expect(info().getAttribute('aria-expanded')).toBe('true');
    fireEvent.mouseEnter(info());
    fireEvent.mouseLeave(info());
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expectForecastNote(forecastNote());
    fireEvent.click(info());
    await waitFor(() => expect(forecastNote()).toBeNull());
  });

  it('keeps a hover-opened note open on click, then closes it on Escape', async () => {
    renderWidget();
    fireEvent.mouseEnter(info());
    fireEvent.mouseMove(info());
    await waitFor(() => expectForecastNote(forecastNote()));
    fireEvent.click(info());
    fireEvent.mouseLeave(info());
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expectForecastNote(forecastNote());
    fireEvent.keyDown(document.body, { key: 'Escape' });
    await waitFor(() => expect(forecastNote()).toBeNull());
  });

  it('keeps a focus-opened note on the first click or Enter and closes it on the next', async () => {
    renderWidget();
    fireEvent.focus(info());
    await waitFor(() => expectForecastNote(forecastNote()));
    // Enter on a focused button arrives as a click.
    fireEvent.click(info());
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expectForecastNote(forecastNote());
    expect(info().getAttribute('aria-expanded')).toBe('true');
    fireEvent.click(info());
    await waitFor(() => expect(forecastNote()).toBeNull());
    expect(info().getAttribute('aria-expanded')).toBe('false');
  });

  it('keeps a focus-opened note when the pointer passes over the button and leaves', async () => {
    renderWidget();
    info().focus();
    fireEvent.focus(info());
    await waitFor(() => expectForecastNote(forecastNote()));
    fireEvent.mouseEnter(info());
    fireEvent.mouseMove(info());
    fireEvent.mouseLeave(info());
    await act(() => new Promise((resolve) => setTimeout(resolve, 50)));
    expectForecastNote(forecastNote());
  });

  it('does not open or close a usage row', () => {
    renderWidget();
    fireEvent.click(info());
    expect(Array.from(document.querySelectorAll('details')).some((row) => row.open)).toBe(false);
  });
});

const nowIso = '2026-10-01T19:00:00Z'; // Thu Oct 1, noon PDT
const at = (iso: string) => new Date(iso).getTime() / 1000;
const weekReset = at('2026-10-06T03:00:00Z'); // Mon Oct 5, 8 PM PDT

function claudeWeek(overrides: Partial<AccountUsageWindow>): AccountUsageWindow {
  return {
    bucket_key: 'limit:weekly_all:weekly_all:weekly',
    window_key: 'weekly_all',
    scope: 'all_models',
    name: 'Claude',
    window: 'Week',
    used_percent: 32,
    duration_minutes: 10080,
    resets_at: weekReset,
    // The app sends daily use (possibly empty) only for the all-models week.
    daily: [],
    ...overrides,
  };
}

function renderClaude(windows: AccountUsageWindow[], checkedAgoSeconds = 60) {
  return render(
    <AccountUsageWidget
      usage={{
        claude: {
          state: 'available',
          issue: null,
          checked_at: Date.now() / 1000 - checkedAgoSeconds,
          windows,
        },
        codex: { state: 'unavailable', issue: 'source_unavailable', checked_at: null, windows: [] },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
}

it('shows a short one-line reset and an on-pace line for the week', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers();
  vi.setSystemTime(new Date(nowIso));
  renderClaude([
    claudeWeek({
      pace: {
        basis: 'window_average',
        projected_percent_at_reset: 67.6,
        expected_percent: 38.1,
        run_out_at: null,
      },
    }),
  ]);
  const summary = screen.getByLabelText('Claude account usage: 68% remaining · Week');
  const sub = summary.querySelector('.xt-account-sub')!;
  expect(sub.firstChild!.textContent).toBe('Week · resets Mon 8 PM');
  const pace = sub.querySelector('.xt-account-pace')!;
  expect(pace.textContent).toBe('On pace · ~68% used by reset');
  expect(pace.classList.contains('is-warning')).toBe(false);
  // Screen readers hear the reset, the pace and how far use is from even
  // use (the even-use mark's text) as the row's description.
  const description = summary
    .getAttribute('aria-describedby')!
    .split(' ')
    .map((id) => document.getElementById(id)!.textContent);
  expect(description).toEqual([
    'Week · resets Mon 8 PM' + 'On pace · ~68% used by reset',
    '6% behind pace',
  ]);
  expect(summary.textContent).not.toContain('Last read');
  expect(summary.textContent).not.toContain('PDT');
  // A reading 12 minutes old says so.
  cleanup();
  renderClaude([claudeWeek({})], 12 * 60);
  expect(
    screen
      .getByLabelText('Claude account usage: 68% remaining · Week')
      .querySelector('.xt-account-sub')!.firstChild!.textContent,
  ).toBe('Week · resets Mon 8 PM · 12 min ago');
  expect(document.querySelector('.xt-account-pace')).toBeNull();
});

it('warns when the week runs out before its reset, and names the week under a fuller session', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers();
  vi.setSystemTime(new Date(nowIso));
  const runOut = at('2026-10-03T22:00:00Z'); // Sat 3 PM PDT
  const week = claudeWeek({
    used_percent: 70,
    pace: {
      basis: 'recent',
      projected_percent_at_reset: 100,
      expected_percent: 38.1,
      run_out_at: runOut,
    },
  });
  renderClaude([week]);
  const pace = screen
    .getByLabelText('Claude account usage: 30% remaining · Week')
    .querySelector('.xt-account-pace')!;
  expect(pace.textContent).toBe('Runs out ~Sat 3 PM');
  expect(pace.classList.contains('is-warning')).toBe(true);
  cleanup();
  const session: AccountUsageWindow = {
    ...claudeWeek({}),
    bucket_key: 'limit:session:session:session',
    window_key: 'session',
    window: 'Session',
    daily: undefined,
    used_percent: 80,
    duration_minutes: null,
    resets_at: at('2026-10-01T20:10:00Z'),
    pace: {
      basis: 'window_average',
      projected_percent_at_reset: 100,
      expected_percent: 38.1,
      run_out_at: at('2026-10-01T19:40:00Z'),
    },
  };
  renderClaude([
    session,
    {
      ...week,
      series: [{ at: Date.now() / 1000 - 60, used_percent: 70 }],
      daily: [{ date: '2026-10-01', used_points: 5, partial: false }],
    },
  ]);
  const summary = screen.getByLabelText('Claude account usage: 20% remaining · Session');
  expect(summary.querySelector('.xt-account-sub')!.firstChild!.textContent).toBe(
    'Session · resets today 1:10 PM',
  );
  expect(summary.querySelector('.xt-account-pace')!.textContent).toBe('Week runs out ~Sat 3 PM');
  // Claude's details list no window rows, not even the week's: they keep
  // only the week's charts and the read time.
  fireEvent.click(summary);
  const detail = summary.closest('details')!.querySelector<HTMLElement>('.xt-account-detail')!;
  expect(within(detail).queryByText('Session')).toBeNull();
  expect(within(detail).queryByText('Week')).toBeNull();
  expect(detail.querySelectorAll('.xt-account-window-head')).toHaveLength(0);
  expect(detail.querySelector('.xt-account-daily')).not.toBeNull();
  expect(within(detail).getByText(/^Read /)).toBeTruthy();
});

it('does not repeat the main week row in the details, and keeps the other windows', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(nowIso));
  const codexWeek = pacedWeek(88, {
    bucket_key: 'codex',
    window_key: 'secondary',
    name: 'Codex',
    series: [{ at: Date.now() / 1000 - 60, used_percent: 88 }],
    daily: [{ date: '2026-10-01', used_points: 5, partial: true }],
  });
  // Codex's session shares the week's bucket key; only the window key differs.
  const codexSession: AccountUsageWindow = {
    ...codexWeek,
    window_key: 'primary',
    window: 'Session',
    used_percent: 20,
    duration_minutes: 300,
    resets_at: at('2026-10-01T22:00:00Z'),
    pace: undefined,
    series: undefined,
    daily: undefined,
  };
  const model: AccountUsageWindow = {
    ...claudeWeek({}),
    bucket_key: 'codex:spark',
    window_key: 'secondary',
    scope: 'model',
    name: 'Spark',
    used_percent: 10,
    daily: undefined,
  };
  render(
    <AccountUsageWidget
      usage={{
        claude: {
          state: 'unavailable',
          issue: 'source_unavailable',
          checked_at: null,
          windows: [],
        },
        codex: {
          state: 'available',
          issue: null,
          checked_at: Date.now() / 1000 - 60,
          windows: [codexSession, codexWeek, model],
        },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  const summary = screen.getByLabelText('Codex account usage: 12% remaining · Week');
  fireEvent.click(summary);
  const detail = summary.closest('details')!.querySelector<HTMLElement>('.xt-account-detail')!;
  // Only the header names the week and its remaining percent.
  expect(within(detail).queryByText('Week')).toBeNull();
  expect(within(detail).queryByText('12% remaining')).toBeNull();
  expect(detail.querySelectorAll('.xt-account-window-head')).toHaveLength(2);
  expect(within(detail).getByText('Session')).toBeTruthy();
  expect(within(detail).getByText('Spark · Week')).toBeTruthy();
  // The week's charts stay, first, above the other windows.
  const charts = detail.firstElementChild!;
  expect(charts.querySelector('.xt-account-burndown')).not.toBeNull();
  expect(charts.querySelector('.xt-account-daily')).not.toBeNull();
  expect(charts.querySelector('.xt-account-window-head')).toBeNull();
  expect(within(detail).getByText(/^Read /)).toBeTruthy();
});

it('says nothing about running out once a stale projection has passed', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers();
  vi.setSystemTime(new Date(nowIso));
  renderClaude(
    [
      claudeWeek({
        used_percent: 70,
        pace: {
          basis: 'recent',
          projected_percent_at_reset: 100,
          expected_percent: 38.1,
          run_out_at: Date.now() / 1000 - 600,
        },
      }),
    ],
    3 * 3600,
  );
  const summary = screen.getByLabelText('Claude account usage: 30% remaining · Week');
  expect(summary.querySelector('.xt-account-pace')).toBeNull();
  expect(summary.textContent).not.toContain('Runs out');
});

it('draws the week by day on a fixed scale, with numbers, unknown and upcoming days', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers();
  vi.setSystemTime(new Date(nowIso));
  renderClaude([
    claudeWeek({
      resets_at: at('2026-10-06T03:00:00Z'), // Mon Oct 5, 8 PM
      daily: [
        { date: '2026-09-28', used_points: 4, partial: false },
        { date: '2026-09-29', used_points: 45, partial: false },
        { date: '2026-09-30', used_points: null, partial: false },
        { date: '2026-10-01', used_points: 5.6, partial: true },
      ],
    }),
  ]);
  const summary = screen.getByLabelText('Claude account usage: 68% remaining · Week');
  fireEvent.click(summary);
  const chart = within(summary.closest('details')!).getByRole('list', {
    name: 'Use per day this week',
  });
  const days = within(chart).getAllByRole('listitem');
  // Each day's label is text inside the item, read by screen readers. The
  // app's days end today; the rest of the window, through the reset's day,
  // is upcoming.
  expect(days.map((day) => day.querySelector('.sr-only')!.textContent)).toEqual([
    'Mon, Sep 28: 4% of the week used',
    'Tue, Sep 29: 45% of the week used',
    'Wed, Sep 30: no reading',
    'Thu, Oct 1: at least 5.6% of the week used',
    'Fri, Oct 2: upcoming',
    'Sat, Oct 3: upcoming',
    'Sun, Oct 4: upcoming',
    'Mon, Oct 5: upcoming',
  ]);
  expect(days[2].getAttribute('aria-label')).toBeNull();
  expect(days.map((day) => day.className)).toEqual([
    'is-known',
    'is-known is-over',
    'is-unknown',
    'is-known is-partial',
    'is-upcoming',
    'is-upcoming',
    'is-upcoming',
    'is-upcoming',
  ]);
  // The number over each bar; a lower bound reads "at least".
  expect(days.map((day) => day.querySelector('.xt-account-daily-value')!.textContent)).toEqual([
    '4',
    '>30',
    '–',
    '≥5',
    '\u00a0',
    '\u00a0',
    '\u00a0',
    '\u00a0',
  ]);
  expect(screen.getByText('Week by day · % of week')).toBeTruthy();
  // A full box is 30 points of the week, on every day: one known day does not
  // fill its box, and a bigger day is clipped (and marked) at the top.
  const height = (day: Element) =>
    (day.querySelector('.xt-account-daily-bar > span') as HTMLElement | null)?.style.height;
  expect(height(days[0])).toBe(`${(4 / 30) * 100}%`);
  expect(height(days[1])).toBe('100%');
  expect(height(days[3])).toBe(`${(5.6 / 30) * 100}%`);
  expect(height(days[2])).toBeUndefined();
  expect(height(days[4])).toBeUndefined();
});

it('lists the rest of the window as upcoming or unread, and labels day values', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  const now = at(nowIso); // Thu Oct 1 noon
  // A reading from Tuesday: Wednesday and today have no reading; Friday on is upcoming.
  const cells = weekDays(
    [{ date: '2026-09-29', used_points: 3, partial: false }],
    at('2026-10-03T18:05:00Z'), // Sat Oct 3, 11:05 AM
    now * 1000,
  );
  expect(cells.map((cell) => [cell.date, cell.kind])).toEqual([
    ['2026-09-29', 'known'],
    ['2026-09-30', 'unknown'],
    ['2026-10-01', 'unknown'],
    ['2026-10-02', 'upcoming'],
    ['2026-10-03', 'upcoming'],
  ]);
  // A reset at local midnight adds no day of its own.
  expect(
    weekDays(
      [{ date: '2026-10-01', used_points: 3, partial: false }],
      at('2026-10-03T07:00:00Z'), // Sat Oct 3, 00:00
      now * 1000,
    ).map((cell) => cell.date),
  ).toEqual(['2026-10-01', '2026-10-02']);
  const value = (used_points: number | null, partial = false) =>
    dayValueLabel({ date: '2026-10-01', used_points, partial });
  expect(value(0.4)).toBe('<1');
  expect(value(0)).toBe('0');
  expect(value(12.5)).toBe('13');
  expect(value(12.9, true)).toBe('≥12');
  // A lower bound under one point says nothing about the day.
  expect(value(0.4, true)).toBe('–');
  expect(value(0, true)).toBe('–');
  // The box holds 30 points: the label and the cut-off mark agree.
  expect(value(30)).toBe('30');
  expect(value(30.1)).toBe('>30');
  expect(value(31, true)).toBe('>30');
  expect(value(null)).toBeNull();
});

// Thu Oct 1 noon PDT is 64 of the week's 168 hours: even use is at 38.1%.
const evenNow = 38.1;

function pacedWeek(used: number, overrides: Partial<AccountUsageWindow> = {}): AccountUsageWindow {
  return claudeWeek({
    used_percent: used,
    pace: {
      basis: 'window_average',
      projected_percent_at_reset: Math.min(100, (used / 64) * 168),
      expected_percent: evenNow,
      run_out_at: null,
    },
    ...overrides,
  });
}

it.each([
  [44, '6% ahead of pace'],
  [35, '3% behind pace'],
  [38.6, 'On pace'],
  [37.2, 'On pace'],
])('marks even use on the week bar (used %s): "%s" on hover and on focus', async (used, text) => {
  // Real clocks: the tooltip opens on its own timers. The week resets in
  // three days; the mark's place comes from the app, not from the clock.
  const week = () => pacedWeek(used, { resets_at: Date.now() / 1000 + 3 * 86_400 });
  renderClaude([week()]);
  const tick = screen.getByRole('button', { name: `Even use mark: ${text}` });
  // The bar fills with what remains, so even use sits at 100% − 38.1%.
  expect(tick.style.left).toBe(`${100 - evenNow}%`);
  const bar = tick.closest('.xt-account-summary-bar')!;
  expect((bar.querySelector('.xt-account-summary-track > span') as HTMLElement).style.width).toBe(
    `${100 - used}%`,
  );
  expect(screen.queryByRole('tooltip')).toBeNull();
  fireEvent.mouseEnter(tick);
  fireEvent.mouseMove(tick);
  expect((await screen.findByRole('tooltip')).textContent).toBe(text);
  cleanup();
  renderClaude([week()]);
  fireEvent.focus(screen.getByRole('button', { name: `Even use mark: ${text}` }));
  expect((await screen.findByRole('tooltip')).textContent).toBe(text);
});

it('shows the even-use mark only while the pace line is shown, and only on the week bar', () => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(nowIso));
  const tick = () => document.querySelector('.xt-account-pace-tick');
  renderClaude([pacedWeek(44)]);
  expect(tick()).not.toBeNull();
  cleanup();
  // No pace (too early in the week, nothing used, or no history).
  renderClaude([claudeWeek({ used_percent: 44 })]);
  expect(document.querySelector('.xt-account-pace')).toBeNull();
  expect(tick()).toBeNull();
  cleanup();
  // A stale run-out time that has passed hides the pace line, and the mark.
  renderClaude(
    [
      pacedWeek(70, {
        pace: {
          basis: 'recent',
          projected_percent_at_reset: 100,
          expected_percent: evenNow,
          run_out_at: Date.now() / 1000 - 600,
        },
      }),
    ],
    3 * 3600,
  );
  expect(document.querySelector('.xt-account-pace')).toBeNull();
  expect(tick()).toBeNull();
  cleanup();
  // The main bar is a fuller session: the week's pace line stays, the mark
  // does not go on the session's bar.
  const session: AccountUsageWindow = {
    ...claudeWeek({}),
    bucket_key: 'limit:session:session:session',
    window_key: 'session',
    window: 'Session',
    daily: undefined,
    used_percent: 80,
    duration_minutes: null,
    resets_at: at('2026-10-01T20:10:00Z'),
  };
  renderClaude([session, pacedWeek(44)]);
  expect(document.querySelector('.xt-account-sub .xt-account-pace')!.textContent).toContain('Week');
  expect(tick()).toBeNull();
  // Without the mark, the row's description has no gap text either.
  expect(document.querySelector('summary')!.getAttribute('aria-describedby')).not.toContain(' ');
  expect(document.querySelector('summary .sr-only')).toBeNull();
});

it('does not open or close the row when the mark is clicked or pressed', () => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(nowIso));
  renderClaude([pacedWeek(44)]);
  const tick = screen.getByRole('button', { name: 'Even use mark: 6% ahead of pace' });
  const details = tick.closest('details')!;
  fireEvent.click(tick);
  fireEvent.keyDown(tick, { key: 'Enter' });
  fireEvent.keyDown(tick, { key: ' ' });
  fireEvent.keyUp(tick, { key: ' ' });
  expect(details.open).toBe(false);
  // The row itself still opens.
  fireEvent.click(details.querySelector('summary')!);
  expect(details.open).toBe(true);
  fireEvent.click(tick);
  expect(details.open).toBe(true);
});

describe('the week burndown', () => {
  const runOut = at('2026-10-03T22:00:00Z'); // Sat 3 PM PDT
  const weekStart = weekReset - 7 * 86_400;
  const chartOf = () => {
    const summary = document.querySelector('summary')!;
    fireEvent.click(summary);
    return summary.closest('details')!.querySelector('.xt-account-burndown')!;
  };

  it('draws the readings, even use, the pace to a run-out dot and the reset', () => {
    vi.stubEnv('TZ', 'America/Los_Angeles');
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(nowIso));
    const now = Date.now() / 1000 - 60;
    renderClaude([
      pacedWeek(44, {
        daily: [{ date: '2026-09-28', used_points: 4, partial: false }],
        pace: {
          basis: 'recent',
          projected_percent_at_reset: 100,
          expected_percent: evenNow,
          run_out_at: runOut,
        },
        series: [
          { at: weekStart + 3600, used_percent: 2 },
          { at: weekStart + 86_400, used_percent: 15 },
          { at: weekStart + 2 * 86_400, used_percent: 30 },
          { at: now, used_percent: 44 },
        ],
      }),
    ]);
    const chart = chartOf();
    // Above the per-day chart.
    expect(
      chart.compareDocumentPosition(document.querySelector('.xt-account-daily')!) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    const svg = within(chart as HTMLElement).getByRole('img', {
      name: '56% remaining. Even use would leave 62%. At this pace it runs out Sat 3 PM.',
    });
    expect(
      svg.querySelector('polyline.is-actual')!.getAttribute('points')!.split(' '),
    ).toHaveLength(4);
    // Even use runs from 100% left at the start to 0% at the reset.
    const even = svg.querySelector('line.is-even')!;
    expect(['x1', 'y1', 'x2', 'y2'].map((name) => Number(even.getAttribute(name)))).toEqual([
      32, 6, 196, 56,
    ]);
    expect(Number(svg.querySelector('line.is-reset')!.getAttribute('x1'))).toBe(196);
    // The pace line ends at the run-out dot, on the 0% line.
    const projection = svg.querySelector('line.is-projection')!;
    const dot = svg.querySelector('circle.is-run-out')!;
    expect(projection.getAttribute('x2')).toBe(dot.getAttribute('cx'));
    expect(Number(dot.getAttribute('cy'))).toBe(56);
    expect(Number(dot.getAttribute('cx'))).toBeLessThan(196);
    const legend = chart.querySelector('.xt-account-burndown-legend')!;
    expect(legend.getAttribute('aria-hidden')).toBe('true');
    expect([...legend.querySelectorAll('li')].map((item) => item.textContent)).toEqual([
      'Remaining',
      'Even use',
      'This pace',
      'Runs out',
      'Reset',
    ]);
    // The y axis is labeled at 100% and 0%; the scale is always 0–100%.
    expect(
      [...svg.querySelectorAll('.xt-account-burndown-axis text.is-y')].map((t) => t.textContent),
    ).toEqual(['100%', '0%']);
    // Each local midnight from Tue to Mon is ticked and named.
    expect(svg.querySelectorAll('.is-day-tick')).toHaveLength(7);
    expect(
      [...svg.querySelectorAll('.xt-account-burndown-axis text.is-day')].map((t) => t.textContent),
    ).toEqual(['T', 'W', 'T', 'F', 'S', 'S', 'M']);
    const thu = [...svg.querySelectorAll('.is-day-tick')][2];
    // Thu 00:00 is 52 hours into the week.
    expect(Number(thu.getAttribute('x1'))).toBeCloseTo(32 + (52 / 168) * 164, 5);
    expect(svg.querySelector('.is-latest-label')).toBeNull();
  });

  it('draws readings spanning under 2% of the week as the latest reading, not a line', () => {
    vi.stubEnv('TZ', 'America/Los_Angeles');
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(nowIso));
    const now = Date.now() / 1000 - 60;
    // The user's case: an hour of readings in a 168-hour week, running out early.
    renderClaude([
      pacedWeek(88, {
        pace: {
          basis: 'window_average',
          projected_percent_at_reset: 100,
          expected_percent: evenNow,
          run_out_at: runOut,
        },
        series: [
          { at: now - 3600, used_percent: 87 },
          { at: now - 1800, used_percent: 87.5 },
          { at: now, used_percent: 88 },
        ],
      }),
    ]);
    const chart = chartOf();
    const svg = chart.querySelector('svg[role="img"]')!;
    expect(svg.querySelector('polyline')).toBeNull();
    const dot = svg.querySelector('circle.is-latest')!;
    expect(dot).not.toBeNull();
    expect(svg.querySelector('.is-latest-label')!.textContent).toBe('12%');
    // The pace line and run-out dot stay.
    expect(svg.querySelector('line.is-projection')!.getAttribute('x1')).toBe(
      dot.getAttribute('cx'),
    );
    expect(svg.querySelector('circle.is-run-out')).not.toBeNull();
    expect(
      [...chart.querySelectorAll('.xt-account-burndown-legend li')].map((item) => item.textContent),
    ).toEqual(['Latest reading', 'Even use', 'This pace', 'Runs out', 'Reset']);
  });

  it('needs two readings spanning at least 2% of the window for a line', () => {
    const week = 7 * 86_400;
    expect(readingsSpanVisible([], 0, week)).toBe(false);
    expect(readingsSpanVisible([{ at: 100 }], 0, week)).toBe(false);
    expect(readingsSpanVisible([{ at: 0 }, { at: 0.019 * week }], 0, week)).toBe(false);
    expect(readingsSpanVisible([{ at: 0 }, { at: 0.02 * week }], 0, week)).toBe(true);
  });

  it('with one reading draws the point and a pace line to the reset, and no history line', () => {
    vi.stubEnv('TZ', 'America/Los_Angeles');
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(nowIso));
    renderClaude([pacedWeek(30, { series: [{ at: Date.now() / 1000 - 60, used_percent: 30 }] })]);
    const chart = chartOf();
    const svg = within(chart as HTMLElement).getByRole('img', {
      name: '70% remaining. Even use would leave 62%. At this pace about 21% is left at the reset.',
    });
    expect(svg.querySelector('polyline')).toBeNull();
    expect(svg.querySelectorAll('circle.is-now')).toHaveLength(1);
    expect(svg.querySelector('circle.is-run-out')).toBeNull();
    const projection = svg.querySelector('line.is-projection')!;
    expect(Number(projection.getAttribute('x2'))).toBe(196);
    expect(svg.querySelector('line.is-even')).not.toBeNull();
    expect(
      [...chart.querySelectorAll('.xt-account-burndown-legend li')].map((item) => item.textContent),
    ).toEqual(['Latest reading', 'Even use', 'This pace', 'Reset']);
  });

  it('with no readings draws only even use and the reset', () => {
    vi.stubEnv('TZ', 'America/Los_Angeles');
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(nowIso));
    renderClaude([claudeWeek({ series: [] })]);
    const chart = chartOf();
    const svg = within(chart as HTMLElement).getByRole('img', {
      name: '68% remaining.',
    });
    expect(svg.querySelector('polyline')).toBeNull();
    expect(svg.querySelector('circle')).toBeNull();
    expect(svg.querySelector('line.is-projection')).toBeNull();
    expect(svg.querySelector('line.is-even')).not.toBeNull();
    expect(svg.querySelector('line.is-reset')).not.toBeNull();
    expect(
      [...chart.querySelectorAll('.xt-account-burndown-legend li')].map((item) => item.textContent),
    ).toEqual(['Even use', 'Reset']);
  });

  it('is not drawn for a window without a series', () => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(nowIso));
    renderClaude([pacedWeek(30)]);
    fireEvent.click(document.querySelector('summary')!);
    expect(document.querySelector('.xt-account-burndown')).toBeNull();
  });
});

it('counts a gap of exactly one point, and calls anything smaller on pace', () => {
  const gap = (used: number) =>
    paceGapLabel(
      claudeWeek({
        used_percent: used,
        pace: {
          basis: 'window_average',
          projected_percent_at_reset: 90,
          expected_percent: 40,
          run_out_at: null,
        },
      }),
    );
  expect(gap(41)).toBe('1% ahead of pace');
  expect(gap(39)).toBe('1% behind pace');
  expect(gap(40.99)).toBe('On pace');
  expect(gap(39.01)).toBe('On pace');
  expect(gap(40)).toBe('On pace');
  // Ahead means using faster than even.
  expect(gap(46.6)).toBe('7% ahead of pace');
  expect(paceGapLabel(claudeWeek({}))).toBeNull();
});

it('marks even use on a Codex week bar too', () => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(nowIso));
  render(
    <AccountUsageWidget
      usage={{
        claude: {
          state: 'unavailable',
          issue: 'source_unavailable',
          checked_at: null,
          windows: [],
        },
        codex: {
          state: 'available',
          issue: null,
          checked_at: Date.now() / 1000 - 60,
          windows: [
            pacedWeek(22, {
              bucket_key: 'codex',
              window_key: 'secondary',
              name: 'Codex',
            }),
          ],
        },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
  const codex = screen.getByLabelText('Codex account usage: 78% remaining · Week');
  const tick = within(codex).getByRole('button', { name: 'Even use mark: 16% behind pace' });
  expect(tick.style.left).toBe(`${100 - evenNow}%`);
  expect(codex.getAttribute('aria-describedby')!.split(' ')).toHaveLength(2);
  // WebKit (the app's web view) reaches a button by Tab only with an explicit tabindex.
  expect(tick.getAttribute('tabindex')).toBe('0');
  expect(document.querySelectorAll('.xt-account-pace-tick')).toHaveLength(1);
});

it('marks a day cut off exactly when its number reads over 30', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(nowIso));
  renderClaude([
    claudeWeek({
      daily: [
        { date: '2026-09-29', used_points: 30, partial: false },
        { date: '2026-09-30', used_points: 30.1, partial: false },
        { date: '2026-10-01', used_points: 0.4, partial: true },
      ],
    }),
  ]);
  fireEvent.click(document.querySelector('summary')!);
  const days = [...document.querySelectorAll('.xt-account-daily li')].slice(0, 3);
  expect(days.map((day) => day.querySelector('.xt-account-daily-value')!.textContent)).toEqual([
    '30',
    '>30',
    '–',
  ]);
  expect(days.map((day) => day.classList.contains('is-over'))).toEqual([false, true, false]);
  // Screen readers still hear the exact value.
  expect(days[1].querySelector('.sr-only')!.textContent).toBe(
    'Wed, Sep 30: 30.1% of the week used',
  );
  expect(days[2].querySelector('.sr-only')!.textContent).toBe(
    'Thu, Oct 1: at least 0.4% of the week used',
  );
});

it('does not repeat a main session row that has no pace of its own', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(nowIso));
  const session: AccountUsageWindow = {
    ...claudeWeek({}),
    bucket_key: 'limit:session:session:session',
    window_key: 'session',
    window: 'Session',
    daily: undefined,
    used_percent: 80,
    duration_minutes: null,
    resets_at: at('2026-10-01T20:10:00Z'),
  };
  renderClaude([
    session,
    pacedWeek(44, {
      series: [{ at: Date.now() / 1000 - 60, used_percent: 44 }],
      daily: [{ date: '2026-10-01', used_points: 5, partial: false }],
    }),
  ]);
  const summary = screen.getByLabelText('Claude account usage: 20% remaining · Session');
  fireEvent.click(summary);
  const detail = summary.closest('details')!.querySelector<HTMLElement>('.xt-account-detail')!;
  // The header says all the session row would, and Claude lists no rows, so
  // the details show only the week's charts and the read time.
  expect(within(detail).queryByText('Session')).toBeNull();
  expect(within(detail).queryByText('20% remaining')).toBeNull();
  expect(within(detail).queryByText('Week')).toBeNull();
  expect(detail.querySelectorAll('.xt-account-window-head')).toHaveLength(0);
  expect(detail.querySelector('.xt-account-burndown')).not.toBeNull();
  expect(detail.querySelector('.xt-account-daily')).not.toBeNull();
  expect(within(detail).getByText(/^Read /)).toBeTruthy();
});

it('lists no session or model rows under a Claude week, but keeps its charts and read time', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(nowIso));
  const week = pacedWeek(44, {
    series: [{ at: Date.now() / 1000 - 60, used_percent: 44 }],
    daily: [{ date: '2026-10-01', used_points: 5, partial: false }],
  });
  const session: AccountUsageWindow = {
    ...claudeWeek({}),
    bucket_key: 'limit:session:session:session',
    window_key: 'session',
    window: 'Session',
    daily: undefined,
    used_percent: 43,
    duration_minutes: null,
    resets_at: at('2026-10-02T01:30:00Z'),
    pace: {
      basis: 'recent',
      projected_percent_at_reset: 100,
      expected_percent: 50,
      run_out_at: at('2026-10-02T01:23:00Z'),
    },
  };
  const model: AccountUsageWindow = {
    ...claudeWeek({}),
    bucket_key: 'limit:weekly_model:fable:weekly',
    window_key: 'weekly_model',
    scope: 'model',
    name: 'Fable',
    used_percent: 19,
    daily: undefined,
  };
  renderClaude([session, week, model]);
  const summary = screen.getByLabelText('Claude account usage: 56% remaining · Week');
  // The header row is unchanged: the week, its bar, reset and pace.
  expect(summary.querySelector('.xt-account-sub')!.firstChild!.textContent).toBe(
    'Week · resets Mon 8 PM',
  );
  expect(summary.querySelector('.xt-account-pace')).not.toBeNull();
  expect(summary.querySelector('.xt-account-summary-track')).not.toBeNull();
  fireEvent.click(summary);
  const detail = summary.closest('details')!.querySelector<HTMLElement>('.xt-account-detail')!;
  expect(within(detail).queryByText('Session')).toBeNull();
  expect(within(detail).queryByText('Fable · Week')).toBeNull();
  expect(within(detail).queryByText('57% remaining')).toBeNull();
  expect(within(detail).queryByText('81% remaining')).toBeNull();
  expect(detail.querySelectorAll('.xt-account-window-head')).toHaveLength(0);
  expect(detail.querySelectorAll('.xt-account-window')).toHaveLength(1);
  expect(detail.querySelector('.xt-account-burndown')).not.toBeNull();
  expect(detail.querySelector('.xt-account-daily')).not.toBeNull();
  expect(within(detail).getByText(/^Read /)).toBeTruthy();
});

function renderCodex(windows: AccountUsageWindow[]) {
  return render(
    <AccountUsageWidget
      usage={{
        claude: {
          state: 'unavailable',
          issue: 'source_unavailable',
          checked_at: null,
          windows: [],
        },
        codex: {
          state: 'available',
          issue: null,
          checked_at: Date.now() / 1000 - 60,
          windows,
        },
      }}
      failed={false}
      refreshing={false}
      onRefresh={vi.fn()}
    />,
  );
}

function codexSessionAndWeek(sessionPace: AccountUsageWindow['pace']) {
  const week = pacedWeek(44, {
    bucket_key: 'codex',
    window_key: 'secondary',
    name: 'Codex',
    series: [{ at: Date.now() / 1000 - 60, used_percent: 44 }],
    daily: [{ date: '2026-10-01', used_points: 5, partial: false }],
  });
  const session: AccountUsageWindow = {
    ...week,
    window_key: 'primary',
    window: 'Session',
    used_percent: 80,
    duration_minutes: 300,
    resets_at: at('2026-10-01T20:10:00Z'),
    pace: sessionPace,
    series: undefined,
    daily: undefined,
  };
  return [session, week];
}

it('keeps a Codex session main row listed when it has its own pace', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(nowIso));
  renderCodex(
    codexSessionAndWeek({
      basis: 'window_average',
      projected_percent_at_reset: 100,
      expected_percent: 38.1,
      run_out_at: at('2026-10-01T19:40:00Z'),
    }),
  );
  const summary = screen.getByLabelText('Codex account usage: 20% remaining · Session');
  // The header's pace line is the week's, so the session keeps its own row.
  expect(summary.querySelector('.xt-account-pace')!.textContent).toMatch(/^Week /);
  fireEvent.click(summary);
  const detail = summary.closest('details')!.querySelector<HTMLElement>('.xt-account-detail')!;
  const sessionRow = within(detail).getByText('Session').closest('.xt-account-window')!;
  expect(sessionRow.querySelector('.xt-account-pace')!.textContent).toBe(
    'Runs out ~today 12:40 PM',
  );
  const weekRow = within(detail).getByText('Week').closest('.xt-account-window')!;
  expect(weekRow.querySelector('.xt-account-burndown')).not.toBeNull();
  expect(weekRow.querySelector('.xt-account-daily')).not.toBeNull();
  expect(within(detail).getByText(/^Read /)).toBeTruthy();
});

it('does not repeat a Codex session main row that has no pace of its own', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(nowIso));
  renderCodex(codexSessionAndWeek(undefined));
  const summary = screen.getByLabelText('Codex account usage: 20% remaining · Session');
  fireEvent.click(summary);
  const detail = summary.closest('details')!.querySelector<HTMLElement>('.xt-account-detail')!;
  // The header says all the session row would; the week keeps its row.
  expect(within(detail).queryByText('Session')).toBeNull();
  expect(within(detail).queryByText('20% remaining')).toBeNull();
  expect(detail.querySelectorAll('.xt-account-window-head')).toHaveLength(1);
  expect(within(detail).getByText('Week').closest('.xt-account-window')).not.toBeNull();
});

it('ticks local midnights across the November clock change', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date('2026-10-30T19:00:00Z'));
  // 168 hours ending Wed Nov 4, 8 PM PST: from Wed Oct 28, 9 PM PDT.
  const reset = at('2026-11-05T04:00:00Z');
  const start = reset - 7 * 86_400;
  renderClaude([claudeWeek({ resets_at: reset, series: [] })]);
  fireEvent.click(document.querySelector('summary')!);
  const ticks = [...document.querySelectorAll('.xt-account-burndown .is-day-tick')].map((tick) =>
    Number(tick.getAttribute('x1')),
  );
  // Midnight is 07:00 UTC under PDT and 08:00 UTC after the change (Nov 1, 2 AM).
  const midnights = [
    '2026-10-29T07:00:00Z',
    '2026-10-30T07:00:00Z',
    '2026-10-31T07:00:00Z',
    '2026-11-01T07:00:00Z',
    '2026-11-02T08:00:00Z',
    '2026-11-03T08:00:00Z',
    '2026-11-04T08:00:00Z',
  ].map((iso) => 32 + ((at(iso) - start) / (7 * 86_400)) * 164);
  expect(ticks).toHaveLength(7);
  ticks.forEach((tick, index) => expect(tick).toBeCloseTo(midnights[index], 5));
  expect(
    [...document.querySelectorAll('.xt-account-burndown text.is-day')].map((t) => t.textContent),
  ).toEqual(['T', 'F', 'S', 'S', 'M', 'T', 'W']);
});

it('keeps the latest reading label off the pace line early in the week', () => {
  vi.stubEnv('TZ', 'America/Los_Angeles');
  vi.useFakeTimers({ toFake: ['Date'] });
  // Tue Sep 29, 4 AM PDT: 8 hours into the week.
  vi.setSystemTime(new Date('2026-09-29T11:00:00Z'));
  const label = (used: number) => {
    cleanup();
    renderClaude([
      claudeWeek({
        used_percent: used,
        pace: {
          basis: 'window_average',
          projected_percent_at_reset: 100,
          expected_percent: 4.8,
          run_out_at: at('2026-10-01T19:00:00Z'),
        },
        series: [{ at: Date.now() / 1000 - 60, used_percent: used }],
      }),
    ]);
    fireEvent.click(document.querySelector('summary')!);
    const svg = document.querySelector('.xt-account-burndown svg')!;
    const dot = svg.querySelector('circle.is-latest')!;
    const text = svg.querySelector('.is-latest-label')!;
    const projection = svg.querySelector('line.is-projection')!;
    // The pace line leaves the dot to the right and downward.
    expect(projection.getAttribute('x1')).toBe(dot.getAttribute('cx'));
    expect(Number(projection.getAttribute('x2'))).toBeGreaterThan(Number(dot.getAttribute('cx')));
    return {
      anchor: text.getAttribute('text-anchor'),
      dx: Number(text.getAttribute('x')) - Number(dot.getAttribute('cx')),
      dy: Number(text.getAttribute('y')) - Number(dot.getAttribute('cy')),
    };
  };
  // Room above the dot: above and to its right, clear of the line below it.
  const above = label(30);
  expect(above.anchor).toBe('start');
  expect(above.dy).toBeLessThan(0);
  // Too near the top: below and to its left; the line runs to the right.
  const below = label(3);
  expect(below.anchor).toBe('end');
  expect(below.dx).toBeLessThan(0);
});
