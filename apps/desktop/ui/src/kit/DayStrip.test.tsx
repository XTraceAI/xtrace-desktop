import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { DayStrip } from './DayStrip';
afterEach(cleanup);
it('renders exactly fourteen labelled cells, cutoff boundaries, zero and unknown states', () => {
  const values = [0, 1, 3, 8, null, NaN, -1, ...Array(7).fill(2)];
  render(
    <DayStrip
      days={values.map((value, index) => ({ label: `Day ${index + 1}`, value }))}
      thresholds={[3, 8]}
    />,
  );
  expect(screen.getAllByRole('img')).toHaveLength(14);
  expect(
    screen
      .getAllByRole('img')
      .slice(0, 4)
      .map((el) => el.getAttribute('data-level')),
  ).toEqual(['0', '1', '2', '3']);
  expect(screen.getByRole('img', { name: 'Day 5: unmeasured' }).getAttribute('data-unknown')).toBe(
    'true',
  );
});
it('rejects missing days and invalid threshold contracts instead of inventing observations', () => {
  expect(() => render(<DayStrip days={[]} thresholds={[3, 8]} />)).toThrow(/fourteen/);
  const days = Array.from({ length: 14 }, (_, i) => ({ label: `Day ${i}`, value: 0 }));
  expect(() => render(<DayStrip days={days} thresholds={[8, 3]} />)).toThrow(/increasing/);
});
