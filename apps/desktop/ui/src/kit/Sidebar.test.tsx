import { cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { Sidebar, type LocalIndexStatus, type SidebarProps } from './Sidebar';
import type { AccountUsage } from '../data/generated/AccountUsage';

vi.mock('./HubPopover', () => ({ HubPopover: () => null }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
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
        resets_at: 1790640000,
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
const props = (): SidebarProps => ({
  activeKey: 'dashboard',
  onNavigate: vi.fn(),
  accountUsage: usage,
  listener: { status: 'off' },
  version: '0.2.3',
  theme: 'dark',
  onToggleTheme: vi.fn(),
});

it('renders controlled navigation, groups, badge and disabled Leaderboard without a router', () => {
  const input = props();
  const view = render(<Sidebar {...input} rulebookCount={3} />);
  for (const name of ['Observe', 'Govern', 'Community'])
    expect(screen.getByRole('heading', { name })).toBeTruthy();
  const nav = screen.getByRole('navigation', { name: 'Main navigation' });
  expect(within(nav).getAllByRole('button')).toHaveLength(5);
  fireEvent.click(screen.getByRole('button', { name: 'Sessions' }));
  expect(input.onNavigate).toHaveBeenCalledWith('sessions');
  expect(screen.getByRole('button', { name: 'Dashboard' }).getAttribute('aria-current')).toBe(
    'page',
  );
  fireEvent.click(screen.getByRole('button', { name: /Leaderboard/ }));
  expect(input.onNavigate).toHaveBeenCalledTimes(1);
  expect(screen.getByLabelText('3 rulebook items')).toBeTruthy();
  view.rerender(<Sidebar {...input} activeKey="sessions" rulebookCount={0} leaderboardEnabled />);
  expect(screen.getByRole('button', { name: 'Sessions' }).getAttribute('aria-current')).toBe(
    'page',
  );
  expect(screen.queryByLabelText('3 rulebook items')).toBeNull();
  fireEvent.click(screen.getByRole('button', { name: 'Leaderboard' }));
  expect(input.onNavigate).toHaveBeenLastCalledWith('leaderboard');
});

it('shows the exhausted weekly limit above the unused session and keeps capture status separate', async () => {
  // Keep this saved provider snapshot before its weekly reset while leaving
  // real timers in place for the dialog's async appearance.
  const checkedAt = usage.claude.checked_at;
  if (checkedAt === null) throw new Error('fixture needs a check time');
  vi.spyOn(Date, 'now').mockReturnValue(checkedAt * 1000);
  const view = render(
    <Sidebar
      {...props()}
      surfaces={[
        { host: 'Codex', surface: 'cli', status: 'capturing' },
        {
          host: 'Codex',
          surface: 'desktop',
          status: 'not-capturing',
          reason: 'No capture receipt',
        },
        { host: 'Codex', surface: 'new-surface', status: 'unknown' },
        { host: 'Legacy host', surface: null, status: 'unknown' },
      ]}
    />,
  );
  expect(
    screen.getByLabelText('Claude account usage: 0% remaining · Weekly · limit reached'),
  ).toBeTruthy();
  expect(screen.getByLabelText('Codex account usage: 74% remaining · Weekly')).toBeTruthy();
  fireEvent.click(
    screen.getByLabelText('Claude account usage: 0% remaining · Weekly · limit reached'),
  );
  // Claude's details list no session or model-specific rows.
  expect(within(view.container).queryByText('Session')).toBeNull();
  expect(within(view.container).queryByText('Example model · Weekly')).toBeNull();
  expect(within(view.container).queryByText('100% remaining')).toBeNull();
  expect(screen.getByText('plugin · off')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Capture status' }));
  await screen.findByRole('dialog', { name: 'Capture by surface' });
  expect(screen.getByText('Codex · desktop').parentElement?.textContent).toContain('Not capturing');
  expect(screen.getByText('Codex · cli').parentElement?.textContent).toContain('Capturing');
  expect(screen.getByText('Codex · new-surface').parentElement?.textContent).toContain('Unknown');
  expect(screen.getByText('Legacy host · unknown surface')).toBeTruthy();
  expect(screen.queryByText(/up to date/i)).toBeNull();
});

it('uses footer/action props, reserves the native top inset and gates Team independently', () => {
  const input = props();
  const settings = vi.fn();
  const view = render(<Sidebar {...input} showTeam topInset={74} />);
  expect((screen.getByRole('button', { name: 'Team' }) as HTMLButtonElement).disabled).toBe(true);
  expect((screen.getByRole('button', { name: 'Settings' }) as HTMLButtonElement).disabled).toBe(
    true,
  );
  expect((screen.getByRole('complementary') as HTMLElement).style.paddingTop).toBe('74px');
  view.rerender(
    <Sidebar
      {...input}
      showTeam
      hubConnected
      teamLabel="Team"
      onSettings={settings}
      listener={{ status: 'listening', port: 47421 }}
      updateLabel="Update available"
    />,
  );
  expect(screen.getByText('plugin · :47421')).toBeTruthy();
  expect(screen.getByText('v0.2.3 · Update available')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Team' }));
  expect(input.onNavigate).toHaveBeenCalledWith('team');
  fireEvent.click(screen.getByRole('button', { name: 'Settings' }));
  expect(settings).toHaveBeenCalledOnce();
  fireEvent.click(screen.getByRole('button', { name: 'Switch to light appearance' }));
  expect(input.onToggleTheme).toHaveBeenCalledOnce();
});

it('keeps missing measurements, listener and capture coverage explicitly unknown', async () => {
  render(
    <Sidebar
      {...props()}
      accountUsage={undefined}
      listener={{ status: 'unknown' }}
      topInset={NaN}
    />,
  );
  expect(screen.getByLabelText('Claude account usage: Reading…')).toBeTruthy();
  expect(screen.getByLabelText('Codex account usage: Reading…')).toBeTruthy();
  fireEvent.click(screen.getByRole('button', { name: 'Capture status' }));
  expect(await screen.findByText('Capture status unknown')).toBeTruthy();
  expect(screen.getByText('plugin · unknown')).toBeTruthy();
  expect(screen.queryByText('Capturing')).toBeNull();
  expect((screen.getByRole('complementary') as HTMLElement).style.paddingTop).toBe('16px');
});

// Synthetic local-index descriptions: the component renders what it is given.
const updating: LocalIndexStatus = {
  label: 'updating',
  title: 'Updating',
  tone: 'live',
  summary: 'The local index is ready and changes are reconciled as they happen.',
  hosts: [
    { host: 'Claude Code', state: 'Complete' },
    { host: 'Cursor', state: 'No local history found' },
  ],
};
const openIndex = async () => {
  fireEvent.click(screen.getByRole('button', { name: /^Local index:/ }));
  return screen.findByRole('dialog', { name: 'Local index and plugin status' });
};

it('describes the local index on the status row and keeps the plugin receiver a separate line', async () => {
  const view = render(<Sidebar {...props()} localIndex={updating} />);
  const trigger = screen.getByRole('button', { name: 'Local index: Updating' });
  expect(trigger.textContent).toBe('index · updating');
  expect(trigger.querySelector('i')?.className).toBe('xt-status-live');
  // The receiver being off is not the row's state and is not shown as one.
  expect(screen.queryByText('plugin · off')).toBeNull();
  expect(screen.queryByRole('button', { name: 'Capture status' })).toBeNull();
  const panel = await openIndex();
  expect(within(panel).getByText('Updating').className).toContain('xt-index-live');
  expect(within(panel).getByText(updating.summary)).toBeTruthy();
  expect(within(panel).getByText('Claude Code').parentElement?.textContent).toBe(
    'Claude CodeComplete',
  );
  expect(within(panel).getByText('Cursor').parentElement?.textContent).toBe(
    'CursorNo local history found',
  );
  expect(within(panel).getByText('Plugin receiver').parentElement?.textContent).toBe(
    'Plugin receiverOff',
  );
  expect(within(panel).getByText('Plugin delivery').parentElement?.textContent).toBe(
    'Plugin deliveryUnknown',
  );
  // An off receiver says nothing about capture, installation or the index.
  expect(panel.textContent).not.toMatch(/not capturing|capturing|install|:\d/i);
  expect(within(panel).getByText('Local history is indexed without the plugin.')).toBeTruthy();
  expect(view.container.querySelector('.xt-status-attention')).toBeNull();
});

it('shows every qualification in full and marks only the host scans the caller flags', async () => {
  render(
    <Sidebar
      {...props()}
      localIndex={{
        label: 'partial?',
        title: 'Last known: Updating · partial',
        tone: 'attention',
        summary: 'The local index is ready and changes are reconciled as they happen.',
        notes: [
          'Last scan not complete for Codex (reader failed).',
          'Live updates are unavailable.',
        ],
        hosts: [
          { host: 'Claude Code', state: 'Complete' },
          { host: 'Codex', state: 'Reader failed', attention: true, reason: 'synthetic reason' },
        ],
      }}
    />,
  );
  const trigger = screen.getByRole('button', {
    name: 'Local index: Last known: Updating · partial',
  });
  expect(trigger.textContent).toBe('index · partial?');
  expect(trigger.querySelector('i')?.className).toBe('xt-status-attention');
  const panel = await openIndex();
  expect(within(panel).getByText('Last scan not complete for Codex (reader failed).')).toBeTruthy();
  expect(within(panel).getByText('Live updates are unavailable.')).toBeTruthy();
  expect(within(panel).getByText('Reader failed').className).toBe('xt-index-attention');
  expect(within(panel).getByText('Complete').className).toBe('');
  expect(within(panel).getByText('synthetic reason')).toBeTruthy();
  expect(
    within(panel).getByText(/A complete scan read the history it found\. It is not plugin capture/),
  ).toBeTruthy();
});

it('shows a listening receiver without a port unless the caller supplies one', async () => {
  const view = render(
    <Sidebar {...props()} listener={{ status: 'listening' }} localIndex={updating} />,
  );
  let panel = await openIndex();
  expect(within(panel).getByText('Plugin receiver').parentElement?.textContent).toBe(
    'Plugin receiverListening',
  );
  expect(panel.textContent).not.toMatch(/:\d/);
  expect(
    within(panel).getByText(/A listening receiver is not proof that a plugin delivers/),
  ).toBeTruthy();
  view.rerender(
    <Sidebar {...props()} listener={{ status: 'listening', port: 43100 }} localIndex={updating} />,
  );
  panel = screen.getByRole('dialog', { name: 'Local index and plugin status' });
  expect(within(panel).getByText('Plugin receiver').parentElement?.textContent).toBe(
    'Plugin receiverListening · :43100',
  );
  view.rerender(<Sidebar {...props()} listener={{ status: 'unknown' }} localIndex={updating} />);
  expect(within(panel).getByText('Plugin receiver').parentElement?.textContent).toBe(
    'Plugin receiverUnknown',
  );
  // Without a local index the row is the plugin listener, and still invents no port.
  view.rerender(<Sidebar {...props()} listener={{ status: 'listening' }} />);
  expect(screen.getByText('plugin · listening')).toBeTruthy();
});

it('keeps supplied capture surfaces their own rows beside an index they say nothing about', async () => {
  render(
    <Sidebar
      {...props()}
      localIndex={{ ...updating, hosts: [] }}
      surfaces={[{ host: 'Codex', surface: 'desktop', status: 'not-capturing' }]}
    />,
  );
  const panel = await openIndex();
  expect(within(panel).getByText('No host scan reported')).toBeTruthy();
  expect(within(panel).getByText('Codex · desktop').parentElement?.textContent).toContain(
    'Not capturing',
  );
  expect(within(panel).queryByText('Plugin delivery')).toBeNull();
  expect(within(panel).getByText('Updating')).toBeTruthy();
});

it('links to Settings from the panel only when the action exists, closing the panel', async () => {
  const settings = vi.fn();
  const view = render(<Sidebar {...props()} localIndex={updating} />);
  let panel = await openIndex();
  expect(within(panel).queryByRole('button', { name: 'Index details in Settings' })).toBeNull();
  view.unmount();
  render(<Sidebar {...props()} localIndex={updating} onSettings={settings} />);
  panel = await openIndex();
  fireEvent.click(within(panel).getByRole('button', { name: 'Index details in Settings' }));
  expect(settings).toHaveBeenCalledOnce();
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
});

it('closes the index panel on Escape and returns focus to its trigger', async () => {
  render(<Sidebar {...props()} localIndex={updating} onSettings={vi.fn()} />);
  const trigger = screen.getByRole('button', { name: 'Local index: Updating' });
  trigger.focus();
  const panel = await openIndex();
  expect(trigger.getAttribute('aria-expanded')).toBe('true');
  fireEvent.keyDown(panel, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  expect(trigger.getAttribute('aria-expanded')).toBe('false');
  await waitFor(() => expect(document.activeElement).toBe(trigger));
});
