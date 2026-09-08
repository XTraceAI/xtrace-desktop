import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { TopBar, type TopBarAction } from './TopBar';

afterEach(cleanup);

it('renders the breadcrumb and only changes the selected range when props change', () => {
  const onRange = vi.fn();
  const view = render(
    <TopBar crumb="Sessions" subcrumb="A sample session" range="7d" onRange={onRange} />,
  );
  expect(screen.getByRole('navigation', { name: 'Breadcrumb' }).textContent).toBe(
    '~/Sessions/A sample session',
  );
  expect(screen.getByText('A sample session').getAttribute('aria-current')).toBe('page');
  fireEvent.click(screen.getByRole('button', { name: '30d' }));
  expect(onRange).toHaveBeenCalledExactlyOnceWith('30d');
  expect(screen.getByRole('button', { name: '7d' }).getAttribute('aria-pressed')).toBe('true');
  expect(screen.getByRole('button', { name: '30d' }).getAttribute('aria-pressed')).toBe('false');
  view.rerender(<TopBar crumb="Dashboard" range="30d" onRange={onRange} />);
  expect(screen.getByRole('button', { name: '30d' }).getAttribute('aria-pressed')).toBe('true');
  expect(screen.getByText('Dashboard').getAttribute('aria-current')).toBe('page');
});

it('omits hidden range and blank actions without leaving focusable controls', () => {
  const view = render(<TopBar crumb="Rulebook" showRange={false} />);
  expect(screen.queryByRole('group', { name: 'Date range' })).toBeNull();
  expect(screen.queryAllByRole('button')).toHaveLength(0);
  view.rerender(<TopBar crumb="Rulebook" showRange={false} actionLabel="  " />);
  expect(view.container.querySelectorAll('button, a, [tabindex]')).toHaveLength(0);
});

it.each<TopBarAction>(['scan', 'copy', 'share'])(
  'dispatches the %s action once without changing the range',
  (action) => {
    const onAction = vi.fn();
    const onRange = vi.fn();
    render(
      <TopBar
        crumb="Dashboard"
        range="14d"
        onRange={onRange}
        actionIcon={action}
        actionLabel={action}
        onAction={onAction}
      />,
    );
    const button = screen.getByRole('button', { name: action });
    fireEvent.click(button);
    expect(onAction).toHaveBeenCalledOnce();
    expect(onRange).not.toHaveBeenCalled();
    expect(button.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
  },
);

it('keeps a visible action disabled until its handler is supplied', () => {
  render(<TopBar crumb="Dashboard" showRange={false} actionLabel="Share" />);
  expect((screen.getByRole('button', { name: 'Share' }) as HTMLButtonElement).disabled).toBe(true);
});
