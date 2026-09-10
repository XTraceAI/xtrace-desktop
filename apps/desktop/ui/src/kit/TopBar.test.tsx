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
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  expect(onRange).toHaveBeenCalledExactlyOnceWith('30d');
  expect(screen.getByRole('radio', { name: '7d' }).getAttribute('aria-checked')).toBe('true');
  expect(screen.getByRole('radio', { name: '30d' }).getAttribute('aria-checked')).toBe('false');
  view.rerender(<TopBar crumb="Dashboard" range="30d" onRange={onRange} />);
  expect(screen.getByRole('radio', { name: '30d' }).getAttribute('aria-checked')).toBe('true');
  expect(screen.getByText('Dashboard').getAttribute('aria-current')).toBe('page');
});

it('omits hidden range and blank actions without leaving focusable controls', () => {
  const view = render(<TopBar crumb="Rulebook" showRange={false} />);
  expect(screen.queryByRole('radiogroup', { name: 'Date range' })).toBeNull();
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

it('keeps the custom picker disabled until wired and only selects it after confirmation', () => {
  const onRange = vi.fn();
  const onCustomRange = vi.fn();
  const view = render(<TopBar crumb="Dashboard" range="7d" onRange={onRange} />);
  expect((screen.getByRole('button', { name: 'Custom range' }) as HTMLButtonElement).disabled).toBe(
    true,
  );
  view.rerender(
    <TopBar crumb="Dashboard" range="7d" onRange={onRange} onCustomRange={onCustomRange} />,
  );
  fireEvent.click(screen.getByRole('button', { name: 'Custom range' }));
  expect(onCustomRange).toHaveBeenCalledOnce();
  expect(onRange).not.toHaveBeenCalled();
  expect(screen.getByRole('radio', { name: '7d' }).getAttribute('aria-checked')).toBe('true');
  view.rerender(
    <TopBar crumb="Dashboard" range="custom" onRange={onRange} onCustomRange={onCustomRange} />,
  );
  expect(screen.getByRole('button', { name: 'Custom range' }).getAttribute('aria-pressed')).toBe(
    'true',
  );
  expect(screen.queryByRole('radio', { checked: true })).toBeNull();
  // The opener remains usable to change an already-confirmed custom range.
  fireEvent.click(screen.getByRole('button', { name: 'Custom range' }));
  expect(onCustomRange).toHaveBeenCalledTimes(2);
  fireEvent.click(screen.getByRole('radio', { name: '30d' }));
  expect(onRange).toHaveBeenCalledExactlyOnceWith('30d');
});
