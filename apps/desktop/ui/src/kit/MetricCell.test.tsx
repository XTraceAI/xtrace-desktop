import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { MetricCell } from './MetricCell';
import { tokens } from './format';
afterEach(cleanup);

it('exposes a missing-usage reason without replacing unmeasured with zero', () => {
  const format = vi.fn(String);
  const view = render(
    <MetricCell value={null} format={format} reason="Cursor Agent CLI transcript has no usage" />,
  );
  for (const value of [null, undefined, NaN, Infinity, '']) {
    view.rerender(
      <MetricCell
        value={value}
        format={format}
        reason="Cursor Agent CLI transcript has no usage"
      />,
    );
    expect(screen.getByText('Unmeasured: Cursor Agent CLI transcript has no usage')).toBeTruthy();
    expect(screen.getByTitle('Cursor Agent CLI transcript has no usage').textContent).toContain(
      '—',
    );
    expect(screen.queryByText('0')).toBeNull();
  }
  expect(format).not.toHaveBeenCalled();
  view.rerender(<MetricCell value={0} format={format} />);
  expect(screen.getByText('0')).toBeTruthy();
  expect(format).toHaveBeenCalledExactlyOnceWith(0);
});

it('formats measured numbers, preserves model text, and accepts size/alignment props', () => {
  const view = render(<MetricCell value={15_100_000} format={tokens} size={24} align="right" />);
  expect(screen.getByText('15.1M').style.fontSize).toBe('24px');
  expect(screen.getByText('15.1M').style.textAlign).toBe('right');
  view.rerender(<MetricCell value="Sample model" />);
  expect(screen.getByText('Sample model')).toBeTruthy();
});
