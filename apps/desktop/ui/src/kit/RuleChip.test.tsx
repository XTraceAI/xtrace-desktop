import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { RuleChip } from './RuleChip';
import { rules } from './rules';

afterEach(cleanup);

it('associates a clearly named definition trigger with current rule text on focus', async () => {
  render(<RuleChip ruleId="M-06" size="sm" />);
  const trigger = screen.getByRole('button', { name: 'Definition M-06' });
  expect(trigger.getAttribute('data-size')).toBe('sm');
  fireEvent.focus(trigger);
  const tooltip = await screen.findByRole('tooltip');
  expect(trigger.getAttribute('aria-describedby')).toBe(tooltip.id);
  expect(tooltip.textContent).toBe(`M-06 · ${rules['M-06']}`);
  fireEvent.blur(trigger, { relatedTarget: document.body });
  await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
});
