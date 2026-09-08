import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { ProgressBar } from './ProgressBar';
afterEach(cleanup);

it('clamps measured values while leaving missing and nonfinite values unmeasured', () => {
  const view = render(<ProgressBar label="Progress" value={-10} />);
  const bar = screen.getByRole('progressbar');
  for (const [value, expected] of [
    [-10, 0],
    [0, 0],
    [50, 50],
    [120, 100],
  ]) {
    view.rerender(<ProgressBar label="Progress" value={value} />);
    expect(bar.getAttribute('aria-valuenow')).toBe(String(expected));
    expect((bar.firstElementChild as HTMLElement).style.width).toBe(`${expected}%`);
  }
  for (const value of [null, NaN, Infinity]) {
    view.rerender(<ProgressBar label="Progress" value={value} />);
    expect(bar.hasAttribute('aria-valuenow')).toBe(false);
    expect(bar.getAttribute('aria-valuetext')).toBe('Unmeasured');
    expect(bar.children.length).toBe(0);
  }
});
it('normalizes oversized segments proportionally without overflowing finite input sums', () => {
  const view = render(
    <ProgressBar
      label="Backtest"
      segments={[
        { label: 'Followed', tone: 'success', value: 120 },
        { label: 'Ignored', tone: 'danger', value: 40 },
      ]}
    />,
  );
  const widths = () =>
    [...screen.getByRole('progressbar').children].map((part) => (part as HTMLElement).style.width);
  expect(widths()).toEqual(['75%', '25%']);
  expect(screen.getByRole('progressbar').getAttribute('aria-valuetext')).toBe(
    'Followed: 75%; Ignored: 25%',
  );
  view.rerender(
    <ProgressBar
      label="Backtest"
      segments={[
        { label: 'A', tone: 'success', value: Number.MAX_VALUE },
        { label: 'B', tone: 'danger', value: Number.MAX_VALUE },
      ]}
    />,
  );
  expect(widths()).toEqual(['50%', '50%']);
});

it('announces distinct equal-total compositions', () => {
  const view = render(
    <ProgressBar
      label="Backtest"
      segments={[
        { label: 'Followed', tone: 'success', value: 80 },
        { label: 'Ignored', tone: 'danger', value: 20 },
      ]}
    />,
  );
  const bar = screen.getByRole('progressbar');
  expect(bar.getAttribute('aria-valuetext')).toBe('Followed: 80%; Ignored: 20%');
  view.rerender(
    <ProgressBar
      label="Backtest"
      segments={[
        { label: 'Followed', tone: 'success', value: 20 },
        { label: 'Ignored', tone: 'danger', value: 80 },
      ]}
    />,
  );
  expect(bar.getAttribute('aria-valuetext')).toBe('Followed: 20%; Ignored: 80%');
});
