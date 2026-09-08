import { cleanup, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { afterEach, expect, it, vi } from 'vitest';
import { SectionCard } from './SectionCard';
vi.mock('./Popover', () => ({
  Popover: ({ id, open, children }: { id: string; open: boolean; children: ReactNode }) => (
    <div id={id} role="tooltip" hidden={!open}>
      {children}
    </div>
  ),
}));
afterEach(cleanup);

it('renders named section, definition, metadata and caller slots with 36/40px headers', () => {
  const view = render(
    <SectionCard
      title="Activity"
      ruleId="M-06"
      meta="Synthetic window"
      right={<button>Export</button>}
      headerHeight={36}
      footer={<span>Footer</span>}
    >
      <p>Content</p>
    </SectionCard>,
  );
  const section = screen.getByRole('region', { name: 'Activity' });
  expect(section.querySelector('header')?.style.height).toBe('36px');
  expect(screen.getByRole('button', { name: 'Definition M-06' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Export' })).toBeTruthy();
  for (const text of ['Synthetic window', 'Content', 'Footer'])
    expect(screen.getByText(text)).toBeTruthy();
  view.rerender(
    <SectionCard title="Activity" padding={20}>
      <p>Content</p>
    </SectionCard>,
  );
  expect(section.querySelector('header')?.style.height).toBe('40px');
  expect((section.querySelector('.xt-section-panel') as HTMLElement).style.padding).toBe('20px');
  expect(screen.queryByText('Footer')).toBeNull();
});
