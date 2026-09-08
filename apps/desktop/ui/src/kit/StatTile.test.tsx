import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import { StatTile } from './StatTile';
import type { MetricIconName } from './metric-icons';
import { rules } from './rules';
vi.mock('./Popover', () => ({
  Popover: ({ id, open, children }: { id: string; open: boolean; children: ReactNode }) => (
    <div id={id} role="tooltip" hidden={!open}>
      {children}
    </div>
  ),
}));
afterEach(cleanup);

it('renders measured values, units, caller-supplied delta tone, aside and current definition', () => {
  const view = render(
    <StatTile
      label="Agent hours"
      ruleId="M-05"
      icon="clock"
      value={62.3}
      unit="h"
      delta={0.18}
      deltaTone="bad"
      aside="Parallel sessions add"
    />,
  );
  expect(screen.getByText('62.3')).toBeTruthy();
  expect(screen.getByText('h')).toBeTruthy();
  expect(screen.getByText('▲18%').getAttribute('data-tone')).toBe('bad');
  expect(screen.getByText('Parallel sessions add')).toBeTruthy();
  const trigger = screen.getByRole('button');
  expect(trigger.querySelector('button')).toBeNull();
  fireEvent.focus(trigger);
  expect(screen.getByRole('tooltip').textContent).toBe(`M-05 · ${rules['M-05']}`);
  view.rerender(
    <StatTile
      label="Tokens"
      ruleId="M-04"
      icon="token"
      value={null}
      delta={0.18}
      reason="Cursor Agent CLI transcript has no usage"
    />,
  );
  expect(screen.getByText('Unmeasured: Cursor Agent CLI transcript has no usage')).toBeTruthy();
  expect(screen.queryByText('▲18%')).toBeNull();
  view.rerender(<StatTile label="Tokens" ruleId="M-04" icon="token" value={0} />);
  expect(screen.getByText('0')).toBeTruthy();
});

it('supports seven metric icon tones and an explicit override without a shared controls dependency', () => {
  const names: MetricIconName[] = ['lanes', 'merge', 'clock', 'bolt', 'msg', 'token', 'shield'];
  const view = render(
    <>
      {names.map((icon) => (
        <StatTile key={icon} label={icon} ruleId="M-06" icon={icon} value={1} />
      ))}
    </>,
  );
  expect(
    [...view.container.querySelectorAll('.xt-metric-icon')].map((icon) =>
      icon.getAttribute('data-tone'),
    ),
  ).toEqual(['info', 'accent', 'success', 'danger', 'warning', 'info', 'success']);
  view.rerender(
    <StatTile label="Override" ruleId="M-06" icon="clock" iconTone="warning" value={1} />,
  );
  expect(view.container.querySelector('.xt-metric-icon')?.getAttribute('data-tone')).toBe(
    'warning',
  );
});
