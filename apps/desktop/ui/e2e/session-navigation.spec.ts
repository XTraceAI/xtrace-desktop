import { expect, test } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };

const lane = fixture.dashboards[0].lanes[0];

test('opens a Dashboard session by keyboard, and offers the list that holds it', async ({
  page,
}) => {
  await page.goto('/dashboard');
  await expect(page.getByTestId('dashboard-summary')).toBeVisible();
  // The lane opens that one session: the detail route names it exactly, and
  // carries the list state that holds it for the page's Back link.
  const link = page.getByRole('link', { name: `Open session ${lane.session_id}` });
  const id = encodeURIComponent(lane.session_id);
  await expect(link).toHaveAttribute('href', `/sessions/${id}?q=${id}&host=${lane.host}&range=7d`);
  // A real link: focus reaches it and Enter follows it, without a mouse.
  await link.focus();
  await expect(link).toBeFocused();
  // The lane name is clipped to its column, so the focus ring has to be drawn
  // inside the link to stay visible; measure it rather than trust the rule.
  const ring = await link.evaluate((element) => {
    const clip = element.closest('.xt-lane-name')!.getBoundingClientRect();
    const box = element.getBoundingClientRect();
    const style = getComputedStyle(element);
    return {
      width: style.outlineWidth,
      offset: style.outlineOffset,
      inside:
        box.left >= clip.left - 0.5 &&
        box.right <= clip.right + 0.5 &&
        box.top >= clip.top - 0.5 &&
        box.bottom <= clip.bottom + 0.5,
    };
  });
  expect(ring.width).toBe('2px');
  expect(parseFloat(ring.offset)).toBeLessThanOrEqual(0);
  expect(ring.inside).toBe(true);
  await page.keyboard.press('Enter');

  // The session itself, measured over the window the Dashboard showed.
  await expect(page).toHaveURL(new RegExp(`/sessions/${id}\\?`));
  await expect(page.getByText('Measured over the last 7 days', { exact: true })).toBeVisible();

  // Back is a list that holds this session, filtered as the link said.
  const back = page.getByRole('link', { name: '← All sessions' });
  await expect(back).toHaveAttribute('href', `/sessions?q=${id}&host=${lane.host}&range=7d`);
  await back.click();
  await expect(page.getByRole('heading', { name: 'Sessions', exact: true })).toBeVisible();
  await expect(page.getByLabel('Search sessions')).toHaveValue(lane.session_id);
  // The compact host menu shows exactly the followed host as selected.
  await expect(
    page.getByRole('button', { name: 'Filter by host' }).locator('.xt-host-glyph-mark'),
  ).toHaveCount(1);
  await expect(page.getByRole('radio', { name: '7d' })).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();

  // The row's own link carries the same list state back into the session.
  await expect(page.getByRole('link', { name: `Open session ${lane.session_id}` })).toHaveAttribute(
    'href',
    `/sessions/${id}?q=${id}&host=${lane.host}&range=7d`,
  );

  // History moves between the session and its list; the list's search is in
  // its address.
  await page.goBack();
  await expect(page.getByText('Measured over the last 7 days', { exact: true })).toBeVisible();
  await page.goForward();
  await expect(page.getByLabel('Search sessions')).toHaveValue(lane.session_id);
  await expect(page.getByText('Session 00000000', { exact: true })).toBeVisible();

  // The search can be edited by hand from there, and the address follows.
  await page.getByLabel('Search sessions').fill('missing');
  await expect(page.getByText('No sessions match these filters.')).toBeVisible();
  await expect(page).toHaveURL(/[?&]q=missing(&|$)/);
});

test('carries the selected range into the session and measures that window', async ({ page }) => {
  await page.goto('/dashboard');
  await expect(page.getByTestId('dashboard-summary')).toBeVisible();
  await page.getByRole('radio', { name: '30d' }).click();
  const link = page.getByRole('link', { name: `Open session ${lane.session_id}` });
  await expect(link).toHaveAttribute('href', /range=30d/);
  await link.click();
  await expect(page.getByRole('radio', { name: '30d' })).toHaveAttribute('aria-checked', 'true');
  await expect(page.getByText(/^Records .* measured over the last 30 days$/)).toBeVisible();
  await expect(page.getByText('Measured over the last 30 days', { exact: true })).toBeVisible();
});
