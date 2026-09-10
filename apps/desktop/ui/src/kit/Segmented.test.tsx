import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { Segmented } from './Segmented';
afterEach(cleanup);
const options = [
  { value: 'gate', label: 'Gate', tone: 'danger' as const },
  { value: 'advise', label: 'Advise' },
  { value: 'paused', label: 'Paused', disabled: true },
];

it('roves keyboard focus past disabled choices while selection stays controlled', async () => {
  const onChange = vi.fn();
  const view = render(
    <Segmented label="Mode" options={options} value="gate" onChange={onChange} />,
  );
  const gate = screen.getByRole('radio', { name: 'Gate' });
  const advise = screen.getByRole('radio', { name: 'Advise' });
  expect(gate.tabIndex).toBe(0);
  expect(advise.tabIndex).toBe(-1);
  act(() => gate.focus());
  fireEvent.keyDown(gate, { key: 'ArrowRight' });
  await waitFor(() => expect(onChange).toHaveBeenLastCalledWith('advise'));
  await waitFor(() => expect(document.activeElement).toBe(advise));
  expect(advise.tabIndex).toBe(0);
  expect(gate.getAttribute('aria-checked')).toBe('true');
  fireEvent.keyDown(advise, { key: 'ArrowRight' });
  await waitFor(() => expect(document.activeElement).toBe(gate));
  fireEvent.keyDown(gate, { key: 'ArrowRight' });
  await waitFor(() => expect(document.activeElement).toBe(advise));
  fireEvent.click(screen.getByText('Paused'));
  expect(onChange).toHaveBeenCalledTimes(2);
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
  expect(screen.getAllByRole('radio').every((item) => (item as HTMLButtonElement).disabled)).toBe(
    true,
  );
  fireEvent.click(screen.getByText('Gate'));
  expect(onChange).not.toHaveBeenCalled();
});

it('keeps an enabled entry when choices are disabled, removed, then restored', () => {
  const onChange = vi.fn();
  const view = render(
    <Segmented label="Mode" options={options} value="gate" onChange={onChange} />,
  );
  const changed = options.map((option) => ({ ...option, disabled: option.value !== 'advise' }));
  view.rerender(<Segmented label="Mode" options={changed} value="gate" onChange={onChange} />);
  expect(screen.getByRole('radio', { name: 'Advise' }).tabIndex).toBe(0);
  expect(
    screen.getAllByRole('radio').every((item) => item.getAttribute('aria-checked') === 'false'),
  ).toBe(true);
  view.rerender(<Segmented label="Mode" options={[options[1]]} value="gate" onChange={onChange} />);
  expect(screen.getByRole('radio', { name: 'Advise' }).tabIndex).toBe(0);
  view.rerender(<Segmented label="Mode" options={options} value="gate" onChange={onChange} />);
  expect(screen.getByRole('radio', { name: 'Gate' }).getAttribute('aria-checked')).toBe('true');
  expect(onChange).not.toHaveBeenCalled();
});
