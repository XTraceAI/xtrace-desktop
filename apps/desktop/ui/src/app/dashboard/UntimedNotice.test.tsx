import { cleanup, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import type { DashboardUntimed } from '../../data/generated/DashboardUntimed';
import { UNTIMED_NOTICE, UNTIMED_SUMMARY, UntimedNotice } from './UntimedNotice';

afterEach(cleanup);

const notice = () => screen.queryByRole('region', { name: 'Untimed indexed history' });
const untimed = (
  records: number,
  by_surface: DashboardUntimed['by_surface'] = [],
): DashboardUntimed => ({ records, by_surface });

it('says nothing when no indexed record is missing a timestamp', () => {
  render(<UntimedNotice untimed={untimed(0)} />);
  expect(notice()).toBeNull();
  // A report that has not arrived is not a claim that the history is clean.
  cleanup();
  render(<UntimedNotice untimed={undefined} />);
  expect(notice()).toBeNull();
});

it('states the one wording and labels the count as all indexed history', () => {
  render(
    <UntimedNotice
      untimed={untimed(3, [
        { host: 'claude', surface: 'cli', records: 2 },
        { host: 'cursor', surface: null, records: 1 },
      ])}
    />,
  );
  const region = notice()!;
  // One compact line, not a banner: the limit and the count read without
  // opening anything, and the explanation sits behind the line's own summary.
  const summary = region.querySelector('details > summary')!;
  expect(summary.textContent).toBe(`Untimed history3 records · ${UNTIMED_SUMMARY}Details`);
  expect(within(region).getByTestId('untimed-summary').textContent).toBe(
    '3 records · outside dated measurements',
  );
  expect(region.querySelector('details')!.open).toBe(false);
  expect(within(region).getByText(UNTIMED_NOTICE)).toBeTruthy();
  expect(UNTIMED_NOTICE).toBe(
    'Some indexed history has no timestamps and cannot contribute to date-based measurements.',
  );
  const count = within(region).getByTestId('untimed-count').textContent ?? '';
  expect(count).toContain('3 records in all indexed history');
  expect(count).toContain('independent of the selected dates');
  expect(count).toContain('any filter on the table');
  // No share of anything: a fraction here would read as capture or coverage.
  expect(region.textContent).not.toMatch(/%/);
  // The history is disclosed, never diagnosed, and never dated for the source.
  expect(region.textContent).not.toMatch(/broken|failing|unhealthy|estimat/i);
});

it('names every host and surface it counted and leaves an unstated surface unknown', () => {
  render(
    <UntimedNotice
      untimed={untimed(7, [
        { host: 'claude', surface: 'cli', records: 4 },
        { host: 'cursor', surface: null, records: 3 },
      ])}
    />,
  );
  const region = notice()!;
  // Keyboard-operable on its own summary; the rows are real list items, not a
  // hover title, so a reader reaches them without a pointer.
  const details = region.querySelector('details')!;
  expect(details.querySelector('summary')?.textContent).toContain('Untimed history');
  // The list opens with the explanation, inside the same disclosure.
  expect(details.contains(within(region).getByTestId('untimed-count'))).toBe(true);
  const rows = within(region)
    .getByRole('list', { name: 'Untimed indexed history by host and surface' })
    .querySelectorAll('li');
  expect([...rows].map((row) => row.textContent)).toEqual([
    'Claude Code · cli4 records',
    'Cursor · unknown surface3 records',
  ]);
});

it('reports one record in the singular and keeps the total equal to its rows', () => {
  render(<UntimedNotice untimed={untimed(1, [{ host: 'codex', surface: 'cli', records: 1 }])} />);
  expect(screen.getByTestId('untimed-count').textContent).toContain('1 record in all');
  cleanup();
  const rows = [
    { host: 'claude', surface: 'cli', records: 1200 },
    { host: 'claude', surface: 'desktop', records: 345 },
  ];
  render(<UntimedNotice untimed={untimed(1545, rows)} />);
  expect(screen.getByTestId('untimed-count').textContent).toContain('1,545 records');
  const listed = [...screen.getByRole('list').querySelectorAll('li')].map((row) => row.textContent);
  expect(listed).toEqual(['Claude Code · cli1,200 records', 'Claude Code · desktop345 records']);
});

it('draws the same line inline, as one item in a page status row', () => {
  render(
    <UntimedNotice untimed={untimed(5, [{ host: 'claude', surface: 'cli', records: 5 }])} inline />,
  );
  const region = notice()!;
  expect(region.getAttribute('data-inline')).toBe('true');
  // The wording, the global count and the disclosure are the Dashboard's own.
  expect(within(region).getByTestId('untimed-summary').textContent).toBe(
    `5 records · ${UNTIMED_SUMMARY}`,
  );
  expect(region.querySelector('details')!.open).toBe(false);
  expect(within(region).getByTestId('untimed-count').textContent).toContain(
    '5 records in all indexed history, independent of the selected dates',
  );
  // Nothing interactive nests inside the summary that opens it.
  expect(region.querySelector('summary')!.querySelector('a, button, input')).toBeNull();
});
