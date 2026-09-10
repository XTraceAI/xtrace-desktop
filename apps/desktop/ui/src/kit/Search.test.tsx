import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { createRef } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import { Search } from './Search';
afterEach(cleanup);

it('labels the real input, supports focus and controlled text without a phantom shortcut', () => {
  const onValueChange = vi.fn();
  const ref = createRef<HTMLInputElement>();
  const view = render(
    <Search
      ref={ref}
      label="Search sessions"
      value=""
      onValueChange={onValueChange}
      placeholder="Find a session"
      className="session-search-input"
      style={{ caretColor: 'red' }}
    />,
  );
  const input = screen.getByRole('searchbox', { name: 'Search sessions' });
  expect(input.className).toBe('session-search-input');
  expect((input as HTMLInputElement).style.caretColor).toBe('red');
  expect(input.closest('label')?.className).toBe('xt-search');
  ref.current?.focus();
  expect(document.activeElement).toBe(input);
  fireEvent.change(input, { target: { value: 'synthetic' } });
  expect(onValueChange).toHaveBeenCalledExactlyOnceWith('synthetic');
  expect((input as HTMLInputElement).value).toBe('');
  expect(document.querySelector('kbd')).toBeNull();
  view.rerender(
    <Search
      ref={ref}
      label="Search sessions"
      value="synthetic"
      onValueChange={onValueChange}
      shortcut="⌘K"
    />,
  );
  expect((input as HTMLInputElement).value).toBe('synthetic');
  expect(document.querySelector('kbd')?.textContent).toBe('⌘K');
});
