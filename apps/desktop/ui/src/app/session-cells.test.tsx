import { cleanup, render } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import type { SessionRow } from '../data/generated/SessionRow';
import { scenarios } from './pr-analytics.synthetic';
import { AgentTime, HandsOff, handsOffReason, indexed, sessionName } from './session-cells';
import { displayTitle } from './session-context';

/**
 * The shared session cells over rows the native drilldown returned: the same
 * words and values the Sessions list shows, whichever page renders them.
 */
afterEach(cleanup);

const rows = scenarios.measured.pr_sessions.flatMap((page) => page.page.rows);
const byId = (id: string) => rows.find((row) => row.id === id)! as SessionRow;

it('names a session by a saved title, else by its short identity', () => {
  expect(sessionName(byId('s-alpha'))).toBe('Session s-alpha');
  expect(sessionName({ ...byId('s-alpha'), title: 'Saved title' })).toBe('Saved title');
});

it('uses the reviewer label only after a native or saved title', () => {
  expect(displayTitle(null, true)).toBe('Automated review');
  expect(displayTitle('Native title', true)).toBe('Native title');
  expect(sessionName({ ...byId('s-alpha'), automated_review: true })).toBe('Automated review');
  expect(sessionName({ ...byId('s-alpha'), automated_review: false })).toBe('Session s-alpha');
});

it('keeps an idle member measured at zero agent time, never unmeasured', () => {
  const idle = byId('s-idle');
  expect(indexed(idle)?.events).toBe(0);
  const { container } = render(<AgentTime row={idle} />);
  expect(container.textContent).toContain('0h00m');
  expect(container.querySelector('.xt-unmeasured')).toBeNull();
});

it('states a hands-off median with its sample, and why one is absent', () => {
  const { container } = render(<HandsOff row={byId('s-alpha')} />);
  expect(container.textContent).toMatch(/minutes, median of 3 stretches/);
  const desk = byId('s-desk');
  expect(desk.hands_off.state).toBe('unmeasured');
  expect(handsOffReason(desk)).toMatch(/^Excluded: claude desktop timestamps are too coarse/);
  const tools = byId('s-tools');
  expect(handsOffReason(tools)).toBe('No hands-off stretch in this window, so there is no median');
});
