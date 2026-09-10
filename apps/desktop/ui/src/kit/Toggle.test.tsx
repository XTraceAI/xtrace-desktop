import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { Toggle } from './Toggle';
afterEach(cleanup);

it('reports the requested state and waits for controlled props', () => {
  const onChange = vi.fn();
  const view = render(<Toggle label="Capture" checked={false} onChange={onChange} />);
  const toggle = screen.getByRole('switch', { name: 'Capture' });
  fireEvent.click(toggle);
  expect(onChange).toHaveBeenCalledExactlyOnceWith(true);
  expect(toggle.getAttribute('aria-checked')).toBe('false');
  view.rerender(<Toggle label="Capture" checked onChange={onChange} disabled />);
  expect(toggle.getAttribute('aria-checked')).toBe('true');
  fireEvent.click(toggle);
  expect(onChange).toHaveBeenCalledTimes(1);
});
