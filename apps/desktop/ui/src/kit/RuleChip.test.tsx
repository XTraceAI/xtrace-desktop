import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import { RuleChip } from './RuleChip';
import { rules } from './rules';

// This unit isolates trigger state/text; native dismissal and focus are tested in WebKit.
vi.mock('./Popover', () => ({
  Popover: ({ id, open, children }: { id: string; open: boolean; children: ReactNode }) => (
    <div id={id} role="tooltip" hidden={!open}>
      {children}
    </div>
  ),
}));
afterEach(cleanup);

it('associates a clearly named definition trigger with current rule text on focus and hover', () => {
  render(<RuleChip ruleId="M-06" size="sm" />);
  const trigger = screen.getByRole('button', { name: 'Definition M-06' });
  expect(trigger.getAttribute('data-size')).toBe('sm');
  fireEvent.focus(trigger);
  const tooltip = screen.getByRole('tooltip');
  expect(trigger.getAttribute('aria-describedby')).toBe(tooltip.id);
  expect(tooltip.textContent).toBe(`M-06 · ${rules['M-06']}`);
  fireEvent.blur(trigger, { relatedTarget: document.body });
  expect(screen.queryByRole('tooltip')).toBeNull();
  fireEvent.pointerEnter(trigger.parentElement!);
  expect(screen.getByRole('tooltip')).toBeTruthy();
  fireEvent.pointerLeave(trigger.parentElement!);
  expect(screen.queryByRole('tooltip')).toBeNull();
});
