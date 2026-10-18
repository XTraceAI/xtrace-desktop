import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { DataSource, TelemetryControls } from '../../data/DataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { TelemetryState } from '../../data/generated/TelemetryState';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';

const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false, invoke: vi.fn() }));

/** A controllable stand-in: nothing is on screen until a test says so. */
class FakeObserver {
  static all: FakeObserver[] = [];
  target: Element | null = null;
  constructor(private readonly callback: IntersectionObserverCallback) {
    FakeObserver.all.push(this);
  }
  observe(target: Element) {
    this.target = target;
  }
  unobserve() {}
  disconnect() {
    this.target = null;
  }
  takeRecords() {
    return [];
  }
  show(ratio: number) {
    if (!this.target) return;
    const entry = { intersectionRatio: ratio, isIntersecting: ratio > 0, target: this.target };
    this.callback([entry as IntersectionObserverEntry], this as unknown as IntersectionObserver);
  }
}
const showAll = (ratio: number) =>
  act(() => {
    for (const observer of FakeObserver.all) observer.show(ratio);
  });

let visibility: DocumentVisibilityState = 'visible';
beforeEach(() => {
  FakeObserver.all = [];
  visibility = 'visible';
  vi.stubGlobal('IntersectionObserver', FakeObserver);
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => visibility,
  });
});
afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  localStorage.clear();
});

type Saved = { preference_on: boolean; disclosure_shown: boolean };

/**
 * A synthetic app store for the two saved facts. The state it answers is a
 * fixed fixture per case, mirroring the Rust decision only as far as these
 * tests need; the real rule is tested in Rust.
 */
function store({
  saved = { preference_on: true, disclosure_shown: false },
  env_off = false,
  configured = true,
  failedTurnOffs = 0,
}: { saved?: Saved; env_off?: boolean; configured?: boolean; failedTurnOffs?: number } = {}) {
  const facts = { ...saved };
  // A Turn off holds for the launch even when saving it fails.
  let held = false;
  const state = (): TelemetryState => {
    const preference_on = facts.preference_on && !held;
    const can_send = facts.disclosure_shown && preference_on && !env_off;
    const status = env_off
      ? 'environment_off'
      : held && facts.preference_on
        ? 'off_until_restart'
        : !configured
          ? 'not_configured'
          : !facts.preference_on
            ? 'turned_off'
            : !facts.disclosure_shown
              ? 'awaiting_disclosure'
              : 'sending';
    return {
      ...facts,
      preference_on,
      enabled: preference_on && !env_off,
      env_off,
      configured,
      can_send,
      can_change: !env_off,
      show_disclosure: status === 'awaiting_disclosure',
      status,
    };
  };
  const controls = {
    read: vi.fn(async () => state()),
    setEnabled: vi.fn(async (enabled: boolean) => {
      if (env_off) throw new Error('telemetry is turned off by XTRACE_TELEMETRY');
      if (!enabled) {
        held = true;
        if (failedTurnOffs > 0) {
          failedTurnOffs -= 1;
          throw new Error('application database write did not report its outcome');
        }
      } else held = false;
      facts.preference_on = enabled;
      return state();
    }),
    acknowledgeDisclosure: vi.fn(async () => {
      facts.disclosure_shown = true;
      return state();
    }),
    openEventList: vi.fn(async () => {}),
  } satisfies TelemetryControls;
  return { controls, facts };
}

function source(telemetry?: TelemetryControls, history = true): DataSource {
  const unused = async () => Promise.reject(new Error('not used by this test'));
  return {
    kind: 'native',
    accountUsage: async () => {
      throw new Error('Account usage unavailable in this test');
    },
    refreshClaudeUsage: async () => {
      throw new Error('Claude refresh unavailable in this test');
    },
    appInfo: async () => ({ ...exported.app_info, had_indexed_history_at_startup: history }),
    dbCounts: async () => exported.db_counts,
    dashboard: async () => exported.dashboards[0],
    tokensByHost: unused,
    today: async () => exported.today,
    sessionsList: unused,
    sessionRow: async () => null,
    sessionStretches: async () => ({ state: 'missing' }),
    sessionTranscript: unused,
    cancelSessionTranscript: async () => {},
    pullRequests: unused,
    refreshPullRequests: unused,
    cancelPullRequestRefresh: unused,
    pullRequestAnalytics: unused,
    pullRequestSessions: unused,
    nativeIndexStatus: async () => exported.native_index,
    subscribe: async () => () => {},
    telemetry,
  };
}

function mount(path: string, data: DataSource) {
  return render(
    <ThemeProvider>
      <DataProvider source={data}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
}

const disclosure = () => screen.findByTestId('telemetry-disclosure');
const settle = () => act(async () => {});

it('does not record the disclosure before it is visibly on screen', async () => {
  const { controls } = store();
  visibility = 'hidden';
  mount('/settings', source(controls));
  const shown = await disclosure();
  // Mounted, but in a hidden window and not yet reported on screen.
  await settle();
  expect(controls.acknowledgeDisclosure).not.toHaveBeenCalled();
  // On screen in a hidden window: still not shown to anyone.
  await showAll(1);
  await settle();
  expect(controls.acknowledgeDisclosure).not.toHaveBeenCalled();
  // The window becomes visible, but the notice is scrolled out of view.
  await showAll(0);
  visibility = 'visible';
  act(() => void document.dispatchEvent(new Event('visibilitychange')));
  await settle();
  expect(controls.acknowledgeDisclosure).not.toHaveBeenCalled();
  // Only a mostly visible notice counts.
  await showAll(0.4);
  await settle();
  expect(controls.acknowledgeDisclosure).not.toHaveBeenCalled();
  await showAll(1);
  await waitFor(() => expect(controls.acknowledgeDisclosure).toHaveBeenCalledOnce());
  // Recorded, the notice stays readable until closed, and is never recorded twice.
  await showAll(1);
  await settle();
  expect(controls.acknowledgeDisclosure).toHaveBeenCalledOnce();
  expect(shown.isConnected).toBe(true);
  fireEvent.click(within(shown.closest('section')!).getByRole('button', { name: 'Got it' }));
  expect(screen.queryByTestId('telemetry-disclosure')).toBeNull();
  // Closing changed nothing that was saved.
  expect(controls.setEnabled).not.toHaveBeenCalled();
});

const optIn = /opted in|opt-in|you agreed|you chose|you accepted|consent|thank/i;

it('shows the pop-up as one flat checklist that never claims an opt-in', async () => {
  const { controls } = store();
  mount('/settings', source(controls));
  const shown = await disclosure();
  // One surface: no section card, header strip or inner panel inside it.
  expect(shown.classList.contains('xt-telemetry-popup')).toBe(true);
  expect(shown.querySelector('.xt-section-card, .xt-section-panel, .xt-section-header')).toBeNull();
  expect(within(shown).getByRole('heading', { level: 2 }).textContent).toBe(
    'Anonymous usage data is on by default',
  );
  // The line under the title names the events.
  expect(shown.querySelector('.xt-telemetry-popup-sub')?.textContent).toBe(
    "Counts app opens and how often it's used, to improve XTrace.",
  );
  const list = (name: string) =>
    within(within(shown).getByRole('heading', { name }).parentElement!)
      .getAllByRole('listitem')
      .map((item) => item.textContent);
  expect(list('Sent')).toEqual([
    'App and macOS version',
    'Coding agents used (Claude, Codex, Cursor)',
    'A random ID made on this Mac',
  ]);
  expect(list('Never sent')).toEqual([
    'Code or file paths',
    'Prompts or conversation text',
    'Repo names, PR links, token counts or costs',
  ]);
  expect(shown.textContent).not.toMatch(optIn);
  const buttons = within(shown)
    .getAllByRole('button')
    .map((button) => button.getAttribute('aria-label') ?? button.textContent);
  // "Got it" leads; Turn off is the quiet one beside it.
  expect(buttons).toEqual(['Got it', 'Turn off']);
  expect(within(shown).getByRole('button', { name: 'Got it' }).dataset.variant).toBe('primary');
  expect(within(shown).getByRole('button', { name: 'Turn off' }).dataset.variant).toBe('ghost');
  expect(buttons.join(' ')).not.toMatch(/accept|agree|allow|enable|keep/i);
  const link = within(shown).getByRole('link', { name: 'Full list' });
  expect(link.getAttribute('href')).toBe(
    'https://github.com/XTraceAI/xtrace-desktop/blob/main/TELEMETRY.md',
  );
  fireEvent.click(link);
  expect(controls.openEventList).toHaveBeenCalledOnce();
});

it('keeps the full disclosure on the welcome page, saying telemetry is on by default', async () => {
  const { controls } = store();
  mount('/first-launch', source(controls, false));
  const shown = await disclosure();
  const text = shown.closest('section')!.textContent ?? '';
  expect(text).toMatch(/Telemetry is on by default/);
  expect(text).toMatch(/Sent with each event/);
  expect(text).toMatch(/Never sent/);
  expect(text).toMatch(/Repository names, file paths, prompts/);
  expect(text).not.toMatch(optIn);
  const link = within(shown).getByRole('link', { name: /TELEMETRY\.md/ });
  fireEvent.click(link);
  expect(controls.openEventList).toHaveBeenCalledOnce();
});

it('turns telemetry off in one click, and the saved choice is what a restart shows', async () => {
  const { controls, facts } = store();
  const first = mount('/settings', source(controls));
  const shown = await disclosure();
  fireEvent.click(within(shown).getByRole('button', { name: 'Turn off' }));
  await waitFor(() => expect(controls.setEnabled).toHaveBeenCalledWith(false));
  expect(await within(shown).findByTestId('telemetry-off')).toBeTruthy();
  expect(facts.preference_on).toBe(false);
  // The Settings switch reads the same saved state.
  const toggle = screen.getByRole('switch', { name: 'Send anonymous usage data' });
  await waitFor(() => expect(toggle.getAttribute('aria-checked')).toBe('false'));
  first.unmount();
  // A restart reads the saved preference again: off, and no notice asks again.
  mount('/settings', source(controls));
  const restarted = await screen.findByRole('switch', { name: 'Send anonymous usage data' });
  await waitFor(() => expect(restarted.getAttribute('aria-checked')).toBe('false'));
  expect(screen.getByTestId('telemetry-status').textContent).toBe('Off');
  expect(screen.queryByTestId('telemetry-disclosure')).toBeNull();
});

it('shows the environment override as off and does not let the switch change it', async () => {
  const { controls } = store({
    saved: { preference_on: true, disclosure_shown: true },
    env_off: true,
  });
  mount('/settings', source(controls));
  const toggle = await screen.findByRole('switch', { name: 'Send anonymous usage data' });
  await waitFor(() => expect(toggle.getAttribute('aria-checked')).toBe('false'));
  expect(toggle.hasAttribute('disabled') || toggle.getAttribute('aria-disabled') === 'true').toBe(
    true,
  );
  expect(screen.getByTestId('telemetry-status').textContent).toMatch(/XTRACE_TELEMETRY/);
  expect(screen.getByTestId('telemetry-env')).toBeTruthy();
  fireEvent.click(toggle);
  await settle();
  expect(controls.setEnabled).not.toHaveBeenCalled();
  expect(screen.queryByTestId('telemetry-disclosure')).toBeNull();
});

it('shows an existing install the notice once in the main window, and welcome shows it itself', async () => {
  // An upgrade: past welcome, not yet shown.
  const upgrade = store();
  const view = mount('/settings', source(upgrade.controls));
  const notice = await disclosure();
  // A floating card over the window's corner, not part of the page's flow:
  // a labelled region that takes no focus when it appears.
  const card = notice.closest('.xt-telemetry-floating');
  expect(card).not.toBeNull();
  expect(card!.closest('.xt-shell-outlet')).not.toBeNull();
  expect(screen.getByRole('region', { name: 'Anonymous usage data' }).contains(notice)).toBe(true);
  expect(card!.contains(document.activeElement)).toBe(false);
  await showAll(1);
  await waitFor(() => expect(upgrade.controls.acknowledgeDisclosure).toHaveBeenCalledOnce());
  view.unmount();
  // Next launch: recorded, so no notice.
  mount('/settings', source(upgrade.controls));
  await screen.findByRole('switch', { name: 'Send anonymous usage data' });
  await settle();
  expect(screen.queryByTestId('telemetry-disclosure')).toBeNull();
  cleanup();
  // A first launch: the welcome page shows it once, without a Close, and
  // the main window adds no second copy.
  const fresh = store();
  mount('/first-launch', source(fresh.controls, false));
  const welcome = await disclosure();
  await settle();
  expect(screen.getAllByTestId('telemetry-disclosure')).toHaveLength(1);
  // Part of the welcome page, not floating over it.
  expect(welcome.closest('.xt-telemetry-inline')?.closest('.xt-welcome')).not.toBeNull();
  expect(welcome.closest('.xt-telemetry-floating')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Close' })).toBeNull();
  expect(screen.queryByRole('button', { name: 'Got it' })).toBeNull();
  await showAll(1);
  await waitFor(() => expect(fresh.controls.acknowledgeDisclosure).toHaveBeenCalledOnce());
});

it('asks for nothing when this build cannot send', async () => {
  const { controls } = store({ configured: false });
  mount('/settings', source(controls));
  const status = await screen.findByTestId('telemetry-status');
  await waitFor(() =>
    expect(status.textContent).toMatch(/this copy of the app does not send telemetry/),
  );
  expect(screen.queryByTestId('telemetry-disclosure')).toBeNull();
  expect(controls.acknowledgeDisclosure).not.toHaveBeenCalled();
});

it('says a Turn off that could not be saved is off only until restart, and offers a retry', async () => {
  const { controls, facts } = store({ failedTurnOffs: 1 });
  mount('/settings', source(controls));
  const shown = await disclosure();
  fireEvent.click(within(shown).getByRole('button', { name: 'Turn off' }));
  const note = await within(shown).findByTestId('telemetry-off-until-restart');
  // The store still says on, and the notice says exactly that.
  expect(facts.preference_on).toBe(true);
  expect(note.textContent).toMatch(/off for now, but turning it off could not be saved/);
  expect(note.textContent).toMatch(/on again when the app restarts/);
  expect(within(shown).queryByTestId('telemetry-off')).toBeNull();
  expect(within(shown).getByRole('alert').textContent).toBe('Telemetry could not be turned off.');
  expect(document.body.textContent).not.toMatch(/saved one/);
  // The retry saves it; then it is the saved choice.
  fireEvent.click(within(note).getByRole('button', { name: 'Try turning off again' }));
  await waitFor(() => expect(facts.preference_on).toBe(false));
  expect(await within(shown).findByTestId('telemetry-off')).toBeTruthy();
  expect(within(shown).queryByTestId('telemetry-off-until-restart')).toBeNull();
});

it('shows Settings the same truthful state after a Turn off that could not be saved', async () => {
  const { controls, facts } = store({
    saved: { preference_on: true, disclosure_shown: true },
    failedTurnOffs: 1,
  });
  mount('/settings', source(controls));
  const toggle = await screen.findByRole('switch', { name: 'Send anonymous usage data' });
  await waitFor(() => expect(toggle.getAttribute('aria-checked')).toBe('true'));
  fireEvent.click(toggle);
  await waitFor(() =>
    expect(screen.getByTestId('telemetry-status').textContent).toBe(
      'Off until the app restarts: turning it off could not be saved',
    ),
  );
  expect(facts.preference_on).toBe(true);
  expect(toggle.getAttribute('aria-checked')).toBe('false');
  const note = screen.getByTestId('telemetry-off-until-restart');
  expect(note.textContent).toMatch(/on again when the app restarts/);
  expect(screen.getByRole('alert').textContent).toBe('The telemetry setting could not be changed.');
  expect(document.body.textContent).not.toMatch(/saved one/);
  fireEvent.click(within(note).getByRole('button', { name: 'Try turning off again' }));
  await waitFor(() => expect(screen.getByTestId('telemetry-status').textContent).toBe('Off'));
  expect(facts.preference_on).toBe(false);
  expect(screen.queryByTestId('telemetry-off-until-restart')).toBeNull();
});

const escape = (prevented = false) =>
  act(() => {
    const event = new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    if (prevented) event.preventDefault();
    document.body.dispatchEvent(event);
  });

it('closes the floating card on Escape, after dialogs and handled keys, and saves nothing', async () => {
  const { controls } = store();
  // A synthetic storage policy, only so Settings can open its real dialog.
  const retention = {
    read: vi.fn(async () => 'metadata_only' as const),
    set: vi.fn(async () => 'metadata_only' as const),
    purge: vi.fn(async () => ({ tables: [], invalidate_content: false })),
  };
  mount('/settings', { ...source(controls), retention });
  const notice = await disclosure();
  expect(notice.closest('.xt-telemetry-floating')).not.toBeNull();
  // Keyboard order follows where the card appears: after the page.
  const page = screen.getByRole('heading', { name: 'Settings', level: 1 });
  expect(page.compareDocumentPosition(notice) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  // An Escape something else already handled is not for the card.
  await escape(true);
  expect(screen.getByTestId('telemetry-disclosure')).toBe(notice);
  // An open dialog takes Escape first; the card stays.
  fireEvent.click(await screen.findByRole('button', { name: 'Delete stored content' }));
  const dialog = await screen.findByRole('dialog');
  await escape();
  await waitFor(() => expect(dialog.isConnected).toBe(false));
  expect(screen.getByTestId('telemetry-disclosure')).toBe(notice);
  // With nothing else open, Escape closes the card like Close.
  await escape();
  await waitFor(() => expect(screen.queryByTestId('telemetry-disclosure')).toBeNull());
  expect(controls.setEnabled).not.toHaveBeenCalled();
  expect(controls.acknowledgeDisclosure).not.toHaveBeenCalled();
  expect(retention.purge).not.toHaveBeenCalled();
});

it('keeps the welcome page card on Escape', async () => {
  const { controls } = store();
  mount('/first-launch', source(controls, false));
  const welcome = await disclosure();
  expect(welcome.closest('.xt-telemetry-inline')).not.toBeNull();
  await escape();
  await settle();
  expect(screen.getByTestId('telemetry-disclosure')).toBe(welcome);
  expect(controls.setEnabled).not.toHaveBeenCalled();
  expect(controls.acknowledgeDisclosure).not.toHaveBeenCalled();
});

it('closes the pop-up on Got it without saving or recording anything, in every state', async () => {
  for (const setup of [
    {},
    { saved: { preference_on: false, disclosure_shown: false } },
    { failedTurnOffs: 1 },
  ] as const) {
    const { controls } = store(setup);
    const view = mount('/settings', source(controls));
    if ('saved' in setup) {
      // Off before it was ever shown: the app asks for no pop-up.
      await screen.findByTestId('telemetry-status');
      await settle();
      expect(screen.queryByTestId('telemetry-disclosure')).toBeNull();
      view.unmount();
      continue;
    }
    const shown = await disclosure();
    if ('failedTurnOffs' in setup) {
      fireEvent.click(within(shown).getByRole('button', { name: 'Turn off' }));
      await within(shown).findByTestId('telemetry-off-until-restart');
      controls.setEnabled.mockClear();
    }
    fireEvent.click(within(shown).getByRole('button', { name: 'Got it' }));
    await waitFor(() => expect(screen.queryByTestId('telemetry-disclosure')).toBeNull());
    expect(controls.setEnabled).not.toHaveBeenCalled();
    expect(controls.acknowledgeDisclosure).not.toHaveBeenCalled();
    view.unmount();
  }
});

it('offers Got it after Turn off, beside the off message', async () => {
  const { controls } = store();
  mount('/settings', source(controls));
  const shown = await disclosure();
  fireEvent.click(within(shown).getByRole('button', { name: 'Turn off' }));
  await within(shown).findByTestId('telemetry-off');
  expect(within(shown).getByRole('heading', { level: 2 }).textContent).toBe(
    'Anonymous usage data is off',
  );
  expect(within(shown).queryByRole('button', { name: 'Turn off' })).toBeNull();
  fireEvent.click(within(shown).getByRole('button', { name: 'Got it' }));
  await waitFor(() => expect(screen.queryByTestId('telemetry-disclosure')).toBeNull());
  expect(controls.setEnabled).toHaveBeenCalledOnce();
  expect(controls.setEnabled).toHaveBeenCalledWith(false);
});
