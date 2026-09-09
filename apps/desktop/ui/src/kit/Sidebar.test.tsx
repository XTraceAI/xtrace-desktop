import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { Sidebar, type SidebarProps } from './Sidebar';

vi.mock('./HubPopover', () => ({ HubPopover: () => null }));
afterEach(cleanup);
const props = (): SidebarProps => ({
  activeKey: 'dashboard',
  onNavigate: vi.fn(),
  hosts: [
    { host: 'claude', tokens: 0, fillPercent: 80 },
    { host: 'codex', tokens: null, fillPercent: 80 },
  ],
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

it('distinguishes measured zero, null tokens, listener off and unknown surfaces', () => {
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
  expect(screen.getByLabelText('Claude Code tokens: 0').textContent).toBe('0');
  expect(
    (view.container.querySelector('.xt-host-claude .xt-host-track > span') as HTMLElement).style
      .width,
  ).toBe('0%');
  expect(screen.getByLabelText('Codex tokens: unmeasured').textContent).toBe('—');
  expect(
    (view.container.querySelector('.xt-host-codex .xt-host-track > span') as HTMLElement).style
      .width,
  ).toBe('0%');
  expect(screen.getByText('plugin · off')).toBeTruthy();
  expect(screen.getByText('Codex · desktop').parentElement?.textContent).toContain('Not capturing');
  expect(screen.getByText('Codex · cli').parentElement?.textContent).toContain('Capturing');
  expect(screen.getByText('Codex · new-surface').parentElement?.textContent).toContain('Unknown');
  expect(screen.getByText('Legacy host · Unknown surface')).toBeTruthy();
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

it('keeps missing measurements, listener and capture coverage explicitly unknown', () => {
  render(<Sidebar {...props()} hosts={[]} listener={{ status: 'unknown' }} topInset={NaN} />);
  expect(screen.getByText('No host measurements')).toBeTruthy();
  expect(screen.getByText('Capture status unknown')).toBeTruthy();
  expect(screen.getByText('plugin · unknown')).toBeTruthy();
  expect(screen.queryByText('Capturing')).toBeNull();
  expect((screen.getByRole('complementary') as HTMLElement).style.paddingTop).toBe('16px');
});
