import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { Segmented } from './Segmented';
afterEach(cleanup);
const options = [
  { value: 'gate', label: 'Gate', tone: 'danger' as const },
  { value: 'advise', label: 'Advise' },
  { value: 'paused', label: 'Paused', disabled: true },
];

it('roves keyboard focus past disabled choices while selection stays controlled', () => {
  const onChange = vi.fn();
  const view = render(
    <Segmented label="Mode" options={options} value="gate" onChange={onChange} />,
  );
  const gate = screen.getByRole('radio', { name: 'Gate' });
  const advise = screen.getByRole('radio', { name: 'Advise' });
  expect(gate.tabIndex).toBe(0);
  expect(advise.tabIndex).toBe(-1);
  fireEvent.keyDown(gate, { key: 'ArrowRight' });
  expect(onChange).toHaveBeenLastCalledWith('advise');
  expect(document.activeElement).toBe(advise);
  expect(advise.tabIndex).toBe(0);
  expect(gate.getAttribute('aria-checked')).toBe('true');
  fireEvent.keyDown(advise, { key: 'ArrowRight' });
  expect(document.activeElement).toBe(gate);
  fireEvent.keyDown(gate, { key: 'End' });
  expect(document.activeElement).toBe(advise);
  fireEvent.click(screen.getByText('Paused'));
  expect(onChange).toHaveBeenCalledTimes(3);
  view.rerender(<Segmented label="Mode" options={options} value="advise" onChange={onChange} />);
  expect(advise.getAttribute('aria-checked')).toBe('true');
});
it('does not leave a tab stop on a disabled or removed selected option', () => {
  const onChange = vi.fn();
  const view = render(
    <Segmented label="Mode" options={options} value="paused" onChange={onChange} />,
  );
  expect(screen.getByText('Gate').tabIndex).toBe(0);
  view.rerender(
    <Segmented label="Mode" options={options} value="gate" onChange={onChange} disabled />,
  );
  expect(screen.getAllByRole('radio').every((item) => item.tabIndex === -1)).toBe(true);
  fireEvent.click(screen.getByText('Gate'));
  expect(onChange).not.toHaveBeenCalled();
});
