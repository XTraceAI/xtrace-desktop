import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { clampFirst, readSplit, roundShare, splitStorageKey, writeSplit } from './layout-split';
import { SplitHandle } from './SplitHandle';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

it('reads the default layout from empty, malformed or out-of-range storage', () => {
  expect(readSplit()).toEqual({ rows: null, columns: null });
  for (const raw of ['{', 'null', '7', '{"rows": 1, "columns": 0}', '{"rows": "0.5"}']) {
    localStorage.setItem(splitStorageKey, raw);
    expect(readSplit()).toEqual({ rows: null, columns: null });
  }
  localStorage.setItem(splitStorageKey, '{"rows": 0.4, "columns": 2}');
  expect(readSplit()).toEqual({ rows: 0.4, columns: null });
});

it('stores one axis beside the other and forgets the key when both are default', () => {
  expect(writeSplit('rows', 0.6)).toBe(true);
  expect(writeSplit('columns', 0.55)).toBe(true);
  expect(JSON.parse(localStorage.getItem(splitStorageKey)!)).toEqual({ rows: 0.6, columns: 0.55 });
  writeSplit('rows', null);
  expect(readSplit()).toEqual({ rows: null, columns: 0.55 });
  writeSplit('columns', null);
  expect(localStorage.getItem(splitStorageKey)).toBeNull();
});

it('treats storage that throws as the default layout and a lost write', () => {
  vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
    throw new Error('denied');
  });
  vi.spyOn(Storage.prototype, 'setItem').mockImplementation(() => {
    throw new Error('denied');
  });
  expect(readSplit()).toEqual({ rows: null, columns: null });
  expect(writeSplit('rows', 0.5)).toBe(false);
});

it('keeps the first pane within both minimums', () => {
  expect(clampFirst(100, 600, 200, 150)).toBe(200);
  expect(clampFirst(500, 600, 200, 150)).toBe(450);
  expect(clampFirst(300, 600, 200, 150)).toBe(300);
  // Too little room for both minimums: the first keeps its own.
  expect(clampFirst(300, 300, 200, 150)).toBe(200);
  expect(roundShare(0.123456)).toBe(0.1235);
});

function Split() {
  return (
    <div data-testid="container">
      <section>first</section>
      <SplitHandle axis="rows" label="Resize the rows" names={['Top', 'Bottom']} />
      <section>second</section>
    </div>
  );
}

it('draws a stored share on its container and resets it from a double-click or Enter', () => {
  localStorage.setItem(splitStorageKey, '{"rows": 0.6}');
  render(<Split />);
  const container = screen.getByTestId('container');
  const handle = screen.getByRole('separator', { name: 'Resize the rows' });
  expect(handle.getAttribute('aria-orientation')).toBe('horizontal');
  expect(handle.tabIndex).toBe(0);
  expect(container.hasAttribute('data-split-rows')).toBe(true);
  expect(container.style.getPropertyValue('--rows-first')).not.toBe('');
  fireEvent.doubleClick(handle);
  expect(container.hasAttribute('data-split-rows')).toBe(false);
  expect(container.style.getPropertyValue('--rows-first')).toBe('');
  expect(localStorage.getItem(splitStorageKey)).toBeNull();

  localStorage.setItem(splitStorageKey, '{"rows": 0.6, "columns": 0.5}');
  cleanup();
  render(<Split />);
  fireEvent.keyDown(screen.getByRole('separator'), { key: 'Enter' });
  expect(screen.getByTestId('container').hasAttribute('data-split-rows')).toBe(false);
  expect(readSplit()).toEqual({ rows: null, columns: 0.5 });
});

it('leaves the default layout alone while nothing has been moved', () => {
  render(<Split />);
  expect(screen.getByTestId('container').hasAttribute('data-split-rows')).toBe(false);
  expect(screen.getByRole('separator').getAttribute('aria-valuetext')).toMatch(/, default layout$/);
  expect(localStorage.getItem(splitStorageKey)).toBeNull();
});
