import { cleanup, render, screen } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { EvidenceDot, KindBadge, StatePill } from './Badge';
afterEach(cleanup);

it('distinguishes evidence and kinds without relying on color alone', () => {
  render(
    <>
      {(['exact', 'sha', 'inferred'] as const).map((evidence) => (
        <EvidenceDot key={evidence} evidence={evidence} />
      ))}
      <KindBadge kind="mcp" />
      <KindBadge kind="hook" />
    </>,
  );
  for (const [evidence, tone] of [
    ['exact', 'success'],
    ['sha', 'info'],
    ['inferred', 'meta'],
  ])
    expect(
      screen.getByRole('img', { name: `${evidence} evidence` }).getAttribute('data-tone'),
    ).toBe(tone);
  expect(screen.getByText('mcp').getAttribute('data-tone')).toBe('info');
  expect(screen.getByText('hook').getAttribute('data-tone')).toBe('success');
});
it('marks a pulse only for explicitly live state', () => {
  render(
    <>
      <StatePill tone="success" live>
        Listening
      </StatePill>
      <StatePill outlined height={20}>
        Off
      </StatePill>
    </>,
  );
  expect(screen.getByText('Listening').getAttribute('data-live')).toBe('true');
  expect(screen.getByText('Off').getAttribute('data-live')).toBe('false');
  expect(screen.getByText('Off').style.paddingInline).toBe('8px');
});
