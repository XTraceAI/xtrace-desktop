import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { SparkBars } from './SparkBars';
afterEach(cleanup);
it('scales paired/single series with visible zero and independently unmeasured values', () => {
  const view = render(
    <SparkBars
      days={[
        { label: 'Day 1', agent: 0, human: null },
        { label: 'Day 2', agent: 5, human: 10 },
      ]}
      max={10}
    />,
  );
  expect(screen.getByRole('img', { name: 'Day 1, agent: 0' }).style.height).toBe('2px');
  expect(screen.getByRole('img', { name: 'Day 1, agent: 0' }).getAttribute('data-zero')).toBe(
    'true',
  );
  expect(
    screen.getByRole('img', { name: 'Day 1, human: unmeasured' }).getAttribute('data-zero'),
  ).toBeNull();
  expect(screen.getByRole('img', { name: 'Day 2, agent: 5' }).style.height).toBe('18px');
  expect(screen.getByRole('img', { name: 'Day 2, human: 10' }).style.height).toBe('36px');
  view.rerender(<SparkBars days={[{ label: 'Day 1', agent: 100 }]} max={10} height={44} single />);
  expect(screen.getByRole('img', { name: 'Day 1, fires: 100' }).style.height).toBe('44px');
});
it('zero/invalid scales and invalid observations never produce invalid CSS', () => {
  const view = render(<SparkBars days={[{ label: 'Day 1', agent: 0, human: NaN }]} max={0} />);
  for (const max of [0, NaN, Infinity, -1]) {
    view.rerender(<SparkBars days={[{ label: 'Day 1', agent: 0, human: NaN }]} max={max} />);
    for (const bar of screen.getAllByRole('img')) expect(bar.style.height).toBe('2px');
  }
});
