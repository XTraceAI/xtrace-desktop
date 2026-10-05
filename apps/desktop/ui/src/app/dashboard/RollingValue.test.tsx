import { render } from '@testing-library/react';
import { expect, it } from 'vitest';
import { continuous } from '../metric-format';
import { places, RollingValue } from './RollingValue';

const keys = (text: string) => places(text).map(({ key }) => key);
const digits = (root: HTMLElement) => [...root.querySelectorAll<HTMLElement>('.xt-roll-digit')];

it('keys each character by its place value, so a digit rolls only within its own place', () => {
  expect(keys('9.8')).toEqual(['i0', 'point', 'f1d']);
  expect(keys('10.2')).toEqual(['i1', 'i0', 'point', 'f1d']);
  // A value gaining a decimal keeps its integer digits where they were.
  expect(keys('12')).toEqual(['i1', 'i0']);
  expect(keys('12.4')).toEqual(['i1', 'i0', 'point', 'f1d']);
  // Group separators and the below-scale mark are keyed apart from any digit.
  expect(keys('1,234.5')).toEqual(['i3', 'i3,', 'i2', 'i1', 'i0', 'point', 'f1d']);
  expect(keys('<0.1')).toEqual(['i1<', 'i0', 'point', 'f1d']);
});

it('reads as exactly the formatted value from the first frame, with nothing counted up to it', () => {
  const { container } = render(<RollingValue text={continuous(1234.46)} />);
  const value = container.querySelector<HTMLElement>('.xt-roll')!;
  // The text is the value once, whole; the drawn face is hidden from assistive technology and
  // holds no text of its own, so no digit stream can be read or copied.
  expect(value.textContent).toBe('1,234.5');
  expect(value.querySelector('.sr-only')!.textContent).toBe('1,234.5');
  const face = value.querySelector<HTMLElement>('.xt-roll-face')!;
  expect(face.getAttribute('aria-hidden')).toBe('true');
  expect(face.textContent).toBe('');
  // Every digit draws its own final figure; no element holds a figure the value does not have.
  expect(digits(value).map((digit) => digit.dataset.digit)).toEqual(['1', '2', '3', '4', '5']);
  expect(
    [...face.querySelectorAll<HTMLElement>('.xt-roll-char')].map((c) => c.dataset.char),
  ).toEqual([',', '.']);
  expect(digits(value)[2]!.style.getPropertyValue('--digit')).toBe('3');
  // No live region: a change is read when the value is, not announced digit by digit.
  expect(container.querySelector('[aria-live]')).toBeNull();
});

it('keeps a changed digit its element, so the new figure rolls from the old one, and replaces the rest', () => {
  const { container, rerender } = render(<RollingValue text="9.8" />);
  const [units, tenths] = digits(container);
  rerender(<RollingValue text="10.2" />);
  const after = digits(container);
  expect(container.textContent).toBe('10.2');
  // The units and tenths are the same elements with new figures; the tens is new.
  expect(after[1]).toBe(units);
  expect(after[2]).toBe(tenths);
  expect(units!.style.getPropertyValue('--digit')).toBe('0');
  expect(tenths!.style.getPropertyValue('--digit')).toBe('2');
  // A change before the roll finishes retargets the same elements to the latest value.
  rerender(<RollingValue text="10.7" />);
  rerender(<RollingValue text="11.3" />);
  expect(container.textContent).toBe('11.3');
  expect(digits(container)[2]).toBe(tenths);
  expect(tenths!.style.getPropertyValue('--digit')).toBe('3');
});
