import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { BrandMark } from './BrandMark';

afterEach(cleanup);
it('uses the approved mark at each supported size, with accessible standalone text', () => {
  for (const size of [20, 22, 26, 34] as const) {
    const view = render(<BrandMark size={size} showLabel={false} />);
    const image = screen.getByRole('img', { name: 'XTrace' });
    expect(image.getAttribute('src')).toBe('/sidebar-mark.png');
    expect(image.getAttribute('width')).toBe(String(size));
    expect(image.getAttribute('height')).toBe(String(size));
    view.unmount();
  }
  render(<BrandMark />);
  expect(screen.getByText('XTrace')).toBeTruthy();
  expect(screen.queryByRole('img')).toBeNull();
});
