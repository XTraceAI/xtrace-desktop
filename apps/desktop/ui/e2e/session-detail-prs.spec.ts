import { expect, test } from '@playwright/test';
import fixture from '../fixtures/F1.json' with { type: 'json' };

// F1's one session with recorded links: exact, commit and inferred evidence.
const row = fixture.sessions[0].rows.find((candidate) => candidate.pr_links.length > 1)!;
const id = encodeURIComponent(row.id);
const evidence = { exact: 'exact', sha: 'commit', inferred: 'inferred' } as const;

for (const size of [
  { width: 1120, height: 720 },
  { width: 1440, height: 900 },
]) {
  for (const scheme of ['dark', 'light'] as const) {
    test(`names every linked PR in the heading at ${size.width}×${size.height} ${scheme}`, async ({
      page,
    }, info) => {
      await page.setViewportSize(size);
      await page.emulateMedia({ colorScheme: scheme });
      await page.goto(`/sessions/${id}?range=7d`);
      const list = page.getByRole('list', { name: 'Linked PRs' });
      await expect(list).toBeVisible();

      // Every recorded link, in order, each with its own evidence in its name.
      const badges = list.getByRole('img', { name: /linked pull request/ });
      await expect(badges).toHaveCount(row.pr_links.length);
      for (const [index, link] of row.pr_links.entries()) {
        const badge = badges.nth(index);
        const name = `${link.repository}#${link.number}`;
        await expect(badge).toHaveAccessibleName(
          `${name}, linked pull request, ${evidence[link.confidence as keyof typeof evidence]} evidence`,
        );
        await expect(badge).toHaveText(name);
        await expect(badge).toHaveAttribute('data-confidence', link.confidence);
      }

      // Nothing to follow and nothing to stop on: the badges are not links,
      // and the keyboard passes over them from Back to the page's controls.
      await expect(list.getByRole('link')).toHaveCount(0);
      const back = page.getByRole('link', { name: '← All sessions' });
      await back.focus();
      await page.keyboard.press('Tab');
      expect(
        await page.evaluate(() => document.activeElement?.closest('.xt-session-detail-prs')),
      ).toBeNull();

      // The badges read whole, inside the page, without widening it.
      const measured = await list.evaluate((element) => {
        const bounds = element.getBoundingClientRect();
        const names = [...element.querySelectorAll('.xt-session-detail-pr-name')];
        return {
          inside: bounds.right <= innerWidth + 0.5,
          clipped: names.some((name) => name.scrollWidth > name.clientWidth),
          pageFits: document.documentElement.scrollWidth <= innerWidth,
        };
      });
      expect(measured).toEqual({ inside: true, clipped: false, pageFits: true });
      await page
        .locator('.xt-session-detail-heading')
        .screenshot({ path: info.outputPath(`detail-prs-${size.width}-${scheme}.png`) });

      // Many more links than fit on a line wrap onto more lines, and only a
      // name longer than the page itself gives way. F1 records three, so the
      // rest are copies of the drawn badges.
      const wrapped = await list.evaluate((element) => {
        const first = element.querySelector('li')!;
        for (let copy = 0; copy < 12; copy += 1) element.append(first.cloneNode(true));
        const long = first.cloneNode(true) as HTMLElement;
        long.querySelector('.xt-session-detail-pr-name')!.textContent =
          `${'very-long-name-'.repeat(20)}#1`;
        element.append(long);
        const tops = new Set(
          [...element.querySelectorAll('li')].map((li) =>
            Math.round(li.getBoundingClientRect().top),
          ),
        );
        return {
          lines: tops.size,
          inside: element.getBoundingClientRect().right <= innerWidth + 0.5,
          longInside:
            long.getBoundingClientRect().right <= element.getBoundingClientRect().right + 0.5,
          pageFits: document.documentElement.scrollWidth <= innerWidth,
        };
      });
      expect(wrapped.lines).toBeGreaterThan(1);
      expect(wrapped).toMatchObject({ inside: true, longInside: true, pageFits: true });

      // Back still returns to the list.
      await back.click();
      await expect(page.getByRole('heading', { name: 'Sessions', exact: true })).toBeVisible();
    });
  }
}
