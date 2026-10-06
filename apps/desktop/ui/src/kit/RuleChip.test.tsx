import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { act } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import { RuleChip } from './RuleChip';
import { RULE_OPEN_DELAY_MS } from './RulePopover';
import { ruleSummary } from './rules';

afterEach(cleanup);

it('associates a clearly named definition trigger with current rule text on focus', async () => {
  render(<RuleChip ruleId="M-06" size="sm" />);
  const trigger = screen.getByRole('button', { name: 'Definition M-06' });
  expect(trigger.getAttribute('data-size')).toBe('sm');
  trigger.focus();
  fireEvent.focus(trigger);
  const tooltip = await screen.findByRole('tooltip');
  expect(trigger.getAttribute('aria-describedby')).toBe(tooltip.id);
  // Focus opens at once; the hover delay is for the pointer only.
  expect(tooltip.textContent).toBe(ruleSummary('M-06'));
  expect(tooltip.textContent).not.toContain('M-06');
  fireEvent.keyDown(trigger, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  expect(document.activeElement).toBe(trigger);
});

it('opens the short plain-language definition after a hover delay, not on a pass', async () => {
  render(<RuleChip ruleId="M-06" />);
  const trigger = screen.getByRole('button', { name: 'Definition M-06' });
  const hoveredAt = performance.now();
  fireEvent.mouseEnter(trigger);
  fireEvent.mouseMove(trigger);
  expect(screen.queryByRole('tooltip')).toBeNull();
  const tooltip = await screen.findByRole('tooltip');
  expect(performance.now() - hoveredAt).toBeGreaterThanOrEqual(RULE_OPEN_DELAY_MS - 50);
  expect(tooltip.textContent).toBe(ruleSummary('M-06'));
  expect(tooltip.textContent).toContain('average');
  expect(tooltip.textContent).not.toContain('M-06');
});

it('does not open when the pointer passes over and leaves before the delay', async () => {
  render(<RuleChip ruleId="M-06" />);
  const trigger = screen.getByRole('button', { name: 'Definition M-06' });
  fireEvent.mouseEnter(trigger);
  fireEvent.mouseMove(trigger);
  fireEvent.mouseLeave(trigger);
  await new Promise((resolve) => setTimeout(resolve, RULE_OPEN_DELAY_MS + 100));
  expect(screen.queryByRole('tooltip')).toBeNull();
});

it('opens on keyboard focus at once, without the hover delay', () => {
  vi.useFakeTimers();
  try {
    render(<RuleChip ruleId="M-06" />);
    const trigger = screen.getByRole('button', { name: 'Definition M-06' });
    act(() => {
      trigger.focus();
      fireEvent.focus(trigger);
    });
    act(() => vi.advanceTimersByTime(50));
    expect(screen.getByRole('tooltip').textContent).toBe(ruleSummary('M-06'));
  } finally {
    vi.useRealTimers();
  }
});
