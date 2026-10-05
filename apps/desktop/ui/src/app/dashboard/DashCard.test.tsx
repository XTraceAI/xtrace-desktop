import { cleanup, render, screen, within } from '@testing-library/react';
import { afterEach, expect, it } from 'vitest';
import { DashCard } from './DashCard';
afterEach(cleanup);

it('is flat by default: the children follow the header on the one surface', () => {
  render(
    <DashCard title="Caught by your rules" rule="R-05" actions={<button>Act</button>}>
      <p>Body</p>
    </DashCard>,
  );
  const card = screen.getByRole('region', { name: 'Caught by your rules' });
  expect(card.hasAttribute('data-layered')).toBe(false);
  expect(card.querySelector('.xt-section-panel')).toBeNull();
  const [header, body] = [...card.children] as HTMLElement[];
  expect(header.tagName).toBe('HEADER');
  expect(within(header).getByRole('heading', { level: 2, name: 'Caught by your rules' })).toBe(
    header.firstElementChild,
  );
  expect(
    within(header).getByRole('button', { name: 'Caught by your rules definition' }),
  ).toBeTruthy();
  expect(within(header).getByRole('button', { name: 'Act' })).toBeTruthy();
  expect(body.textContent).toBe('Body');
});

it('layered: the header stays on the card and the children sit in the kit section panel', () => {
  render(
    <DashCard title="Environment" rule="M-17" layered actions={<span>Pill</span>}>
      <p>Body</p>
      <p>More</p>
    </DashCard>,
  );
  const card = screen.getByRole('region', { name: 'Environment' });
  expect(card.hasAttribute('data-layered')).toBe(true);
  const [header, panel, ...rest] = [...card.children] as HTMLElement[];
  expect(rest).toEqual([]);
  expect(header.tagName).toBe('HEADER');
  expect(within(header).getByRole('button', { name: 'Environment definition' })).toBeTruthy();
  expect(within(header).getByText('Pill')).toBeTruthy();
  // The same panel the kit SectionCard draws, and nothing else between header and children.
  expect(panel.className).toBe('xt-section-panel');
  expect([...panel.children].map((child) => child.textContent)).toEqual(['Body', 'More']);
});
