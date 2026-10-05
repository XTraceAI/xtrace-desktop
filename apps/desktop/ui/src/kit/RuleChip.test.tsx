import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { RuleChip } from './RuleChip';

afterEach(cleanup);

it('associates a clearly named definition trigger with current rule text on focus', async () => {
  render(<RuleChip ruleId="M-06" size="sm" />);
  const trigger = screen.getByRole('button', { name: 'Definition M-06' });
  expect(trigger.getAttribute('data-size')).toBe('sm');
  trigger.focus();
  fireEvent.focus(trigger);
  const tooltip = await screen.findByRole('tooltip');
  expect(trigger.getAttribute('aria-describedby')).toBe(tooltip.id);
  expect(tooltip.textContent).toContain('peak overlap');
  expect(tooltip.textContent).not.toContain('M-06');
  fireEvent.keyDown(trigger, { key: 'Escape' });
  await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
  expect(document.activeElement).toBe(trigger);
});

it('opens the plain-language definition on hover', async () => {
  render(<RuleChip ruleId="M-06" />);
  const trigger = screen.getByRole('button', { name: 'Definition M-06' });
  fireEvent.mouseEnter(trigger);
  fireEvent.mouseMove(trigger);
  const tooltip = await screen.findByRole('tooltip');
  expect(tooltip.textContent).toContain('mean');
  expect(tooltip.textContent).not.toContain('M-06');
});
