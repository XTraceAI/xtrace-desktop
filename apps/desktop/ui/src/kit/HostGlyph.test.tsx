import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { HostGlyph } from './HostGlyph';
import { hostName } from './hosts';

afterEach(cleanup);

it('labels each known host explicitly with its own identity', () => {
  render(
    <>
      <HostGlyph host="claude" />
      <HostGlyph host="codex" />
      <HostGlyph host="cursor" />
    </>,
  );
  // Each glyph is named as every screen names its host.
  for (const [host, label, file] of [
    ['claude', 'Claude Code', 'claude.svg'],
    ['codex', 'Codex', 'codex.webp'],
    ['cursor', 'Cursor', 'cursor.png'],
  ]) {
    expect(hostName(host)).toBe(label);
    const glyph = screen.getByRole('img', { name: label });
    expect(glyph.title).toBe(label);
    expect(glyph.querySelector('img')?.getAttribute('src')).toBe(`/hosts/${file}`);
    expect(glyph.getAttribute('data-host')).toBe(host);
  }
});

it('renders missing and unrecognized hosts honestly, including object-property names', () => {
  const view = render(<HostGlyph />);
  expect(screen.getByRole('img', { name: 'Unknown host' }).textContent).toBe('?');
  for (const host of ['new-host', 'toString', 'constructor', null]) {
    view.rerender(<HostGlyph host={host} />);
    const glyph = screen.getByRole('img', {
      name: host ? `Unknown host: ${host}` : 'Unknown host',
    });
    expect(glyph.getAttribute('data-host')).toBe('unknown');
    expect(glyph.textContent).toBe('?');
    expect(screen.queryByRole('img', { name: /^(Claude|Codex|Cursor)$/ })).toBeNull();
  }
});

it('supports each size and stacking without introducing a focusable element', () => {
  const view = render(
    <>
      {([16, 18, 20, 30] as const).map((size) => (
        <HostGlyph key={size} host="claude" size={size} stacked />
      ))}
    </>,
  );
  expect(screen.getAllByRole('img').map((glyph) => glyph.getAttribute('data-size'))).toEqual([
    '16',
    '18',
    '20',
    '30',
  ]);
  expect(
    screen.getAllByRole('img').every((glyph) => glyph.getAttribute('data-stacked') === 'true'),
  ).toBe(true);
  expect(view.container.querySelectorAll('button, a, [tabindex]')).toHaveLength(0);
});
