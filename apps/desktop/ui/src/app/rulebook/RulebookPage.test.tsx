import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { MemoryRouter } from 'react-router';
import { afterEach, expect, it, vi } from 'vitest';
import fixture from '../../../fixtures/F1.json';
import { DataProvider } from '../../data/DataProvider';
import type { RuleActivityControls } from '../../data/DataSource';
import { FixtureDataSource } from '../../data/FixtureDataSource';
import type { FixtureExport } from '../../data/generated/FixtureExport';
import type { RuleActivityResult } from '../../data/generated/RuleActivityResult';
import { ThemeProvider } from '../../theme/ThemeProvider';
import { AppRoutes } from '../AppRoutes';
import * as synthetic from './rule-activity.synthetic';

// JSON imports widen literal unions; the export is the generated shape.
const exported = fixture as FixtureExport;
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }));
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  localStorage.clear();
});

type Pending = {
  id: string;
  resolve: (result: RuleActivityResult) => void;
  reject: (error: unknown) => void;
};
/** Rule activity controls whose reads end only when a test ends them. */
function controlled() {
  const reads: Pending[] = [];
  const cancels: string[] = [];
  const controls: RuleActivityControls = {
    read: (id) =>
      new Promise((resolve, reject) => {
        reads.push({ id, resolve, reject });
      }),
    cancel: async (id) => {
      cancels.push(id);
    },
  };
  return { controls, reads, cancels };
}
/** The F1 fixture source, answering rule activity through `controls` (or not at all). */
function sourceWith(controls: RuleActivityControls | undefined) {
  const source = new FixtureDataSource(exported);
  Object.defineProperty(source, 'ruleActivity', { value: controls });
  return source;
}
/** Controls that answer every read at once. */
const answering = (result: RuleActivityResult | (() => Promise<RuleActivityResult>)) => {
  const read = vi.fn<RuleActivityControls['read']>(() =>
    typeof result === 'function' ? result() : Promise.resolve(structuredClone(result)),
  );
  const cancel = vi.fn<RuleActivityControls['cancel']>(async () => {});
  return { read, cancel };
};

/** The runtime mounts its screens once every listener has registered. */
const opened = () =>
  waitFor(() => expect(screen.queryByText('Connecting live updates…')).toBeNull());
/** Lets every pending answer land, so a check that nothing changed means it. */
const settled = () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
async function mount(path: string, source = new FixtureDataSource(exported)) {
  const view = render(
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>,
  );
  await opened();
  return view;
}
const page = () => document.querySelector<HTMLElement>('.xt-rulebook')!;
const crumb = () => within(screen.getByRole('navigation', { name: 'Breadcrumb' }));
const status = () => within(page()).getByRole('status');
const tiles = () => [...page().querySelectorAll<HTMLElement>('.xt-rulebook-tiles .xt-stat-tile')];
const tileValue = (index: number) =>
  tiles()[index]!.querySelector('.xt-stat-value')?.textContent ?? '';
const table = (name: string) => within(page()).getByRole('table', { name });
const dataRows = (name: string) =>
  within(table(name))
    .getAllByRole('row')
    .filter((row) => row.classList.contains('xt-data-row'));
/** A row's Rule cell as shown: its group's position, never a raw ID. */
const ruleTitle = (row: HTMLElement) => row.querySelector('.xt-title-cell span')?.textContent;
/** The IDs a row's identity control is described by, which no row shows. */
const identityOf = (row: HTMLElement) => {
  const control = row.querySelector<HTMLElement>('.xt-rulebook-identity')!;
  return document.getElementById(control.getAttribute('aria-describedby')!)?.textContent;
};
/** A row's text as displayed: its hidden identity description left out. */
const shownText = (row: HTMLElement) => {
  const copy = row.cloneNode(true) as HTMLElement;
  copy.querySelectorAll('[hidden]').forEach((hidden) => hidden.remove());
  return copy.textContent;
};
const NUMBERING =
  'Groups are numbered by position in this snapshot, not by rule; numbers may change on Refresh.';
const buttons = () => within(page().querySelector('.xt-rulebook-read')!).getAllByRole('button');
const refreshButton = () => buttons()[0]!;
const cancelButton = () => buttons()[1]!;
const offered = (button: HTMLElement) => button.getAttribute('aria-disabled') !== 'true';
/** No read is running: Refresh is offered and Cancel is not. */
function expectIdle() {
  expect(buttons().map((button) => button.textContent)).toEqual(['Refresh', 'Cancel']);
  expect(offered(refreshButton())).toBe(true);
  expect(offered(cancelButton())).toBe(false);
}
/** A read is running: Cancel is offered and Refresh is not. */
function expectReading() {
  expect(buttons().map((button) => button.textContent)).toEqual(['Refresh', 'Cancel']);
  expect(offered(refreshButton())).toBe(false);
  expect(offered(cancelButton())).toBe(true);
}

/** No page offers a way to act on a rule, or leads to a rule, fire or session. */
function expectReadOnly(root: HTMLElement) {
  expect(within(root).queryAllByRole('checkbox')).toHaveLength(0);
  expect(within(root).queryAllByRole('switch')).toHaveLength(0);
  expect(within(root).queryAllByRole('dialog')).toHaveLength(0);
  expect(
    within(root)
      .queryAllByRole('link')
      .map((link) => link.getAttribute('href')),
  ).toEqual(root.dataset.view === 'overview' ? [] : ['/rulebook']);
  // No session identifier recorded on a row reaches the page.
  expect(root.textContent).not.toMatch(/synthetic-session/);
  expect(root.textContent).not.toMatch(/View all/i);
  // Focus moves through the page in document order and is never held.
  expect(root.querySelectorAll('[tabindex]:not([tabindex="0"]):not([tabindex="-1"])')).toHaveLength(
    0,
  );
}
/** A state that holds no snapshot shows no count, row or window. */
function expectNoCounts(root: HTMLElement) {
  expect(within(root).queryAllByRole('table')).toHaveLength(0);
  for (const tile of tiles()) expect(tile.querySelector('.xt-unmeasured')).toBeTruthy();
  expect(root.querySelector('.xt-rulebook-source')).toBeNull();
  expect(root.querySelectorAll('time')).toHaveLength(0);
}
/**
 * Without a snapshot the Recorded fires tile states what the read bar states,
 * and never says it was not read once a read has ended.
 */
function expectFiresTile(reason: string, aside: string) {
  const tile = tiles()[0]!;
  const unmeasured = tile.querySelector('.xt-unmeasured')!;
  expect(unmeasured.textContent).toBe(`—Unmeasured: ${reason}`);
  expect(unmeasured.getAttribute('title')).toBe(reason);
  expect(tile.querySelector('.xt-stat-aside')?.textContent).toBe(aside);
  expect(tile.textContent).not.toMatch(/not read/i);
}

it('reads the fixture’s unconfigured source on opening and says so, never as zero', async () => {
  await mount('/rulebook');
  const root = page();
  expect(screen.getByRole('heading', { level: 1, name: 'Rulebook' })).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Rulebook' }).getAttribute('aria-current')).toBe(
    'page',
  );
  expect(crumb().getByText('rulebook')).toBeTruthy();
  expect(await within(root).findByText('Source unavailable')).toBeTruthy();
  const detail = root.querySelector('.xt-rulebook-read .xt-rulebook-detail')?.textContent;
  expect(detail).toBe(
    'Default local rulebook source · Its folder is not configured on this device. Nothing was counted.',
  );
  expect(tiles().map((tile) => tile.querySelector('.xt-stat-label')?.textContent)).toEqual([
    'Recorded fires',
    'Blocked',
    'Adherence',
    'Judge overhead',
  ]);
  expectFiresTile('Source unavailable', 'unavailable');
  for (const tile of tiles().slice(1)) {
    expect(tile.querySelector('.xt-unmeasured')?.textContent).toBe(
      '—Unmeasured: Not read by this app',
    );
    expect(tile.querySelector('.xt-stat-aside')?.textContent).toBe('unavailable');
  }
  // A note stands where Proposals stood; groups sit beside the timeline.
  expect(screen.getByText(/Rule proposals and active rules are not read by this app/)).toBeTruthy();
  expect(
    within(root)
      .getAllByRole('heading', { level: 2 })
      .map((heading) => heading.textContent),
  ).toEqual(['Observed groups', 'Recorded timeline']);
  // The read has ended: nothing to wait for.
  expect(within(root).getAllByText('No rows are listed from this read.')).toHaveLength(2);
  expectNoCounts(root);
  expectReadOnly(root);
  expectIdle();
});

it('shows a loaded snapshot’s exact counts, window, groups and timeline', async () => {
  const controls = answering(synthetic.loaded());
  await mount('/rulebook', sourceWith(controls));
  const root = page();
  await waitFor(() => expect(status().textContent).toMatch(/^Read complete/));
  expect(controls.read).toHaveBeenCalledTimes(1);
  // The fixture answers under its own read name; that is not a stale answer.
  expect(controls.read.mock.calls[0]![0]).not.toBe('synthetic-read');
  expect(status().textContent).toBe('Read complete · 4 recorded fires in 4 observed groups');

  // Recorded fires are advise + gate; nothing else is populated.
  expect(tileValue(0)).toBe('4');
  expect(tiles()[0]!.querySelector('.xt-stat-aside')?.textContent).toBe('14d · exact');
  for (const tile of tiles().slice(1))
    expect(tile.querySelector('.xt-stat-aside')?.textContent).toBe('unavailable');

  // The returned window and read instant, exactly as returned; no path.
  const source = root.querySelector('.xt-rulebook-source')!;
  expect(source.textContent).toBe(
    'Default local rulebook source · window 2026-09-09T09:00:00Z to 2026-09-23T09:00:00Z · read 2026-09-23T09:00:00Z · 7 lines scanned',
  );
  const note = root.querySelector('.xt-rulebook-note')!.textContent;
  expect(note).toMatch(/Recorded fires are rows recorded with mode advise or gate/);
  expect(note).toMatch(/suppressed \(1\) and unrecognized-mode \(1\) rows are counted apart/);
  expect(note).toMatch(/not of all rule activity, active rules or outcomes/);
  expect(note).toMatch(/Counts are exact for this snapshot\./);
  expect(note).not.toMatch(/grew/);

  // Groups: exactly the returned (rulebook, rule) pairs, named by position;
  // the unscoped pair says so, and no raw ID is in a row's visible text.
  const groups = dataRows('Observed rule groups');
  expect(groups.map(ruleTitle)).toEqual([
    'Observed group 1',
    'Observed group 2',
    'Observed group 3',
    'Observed group 4',
  ]);
  expect(groups.map(identityOf)).toEqual([
    'Rule ID synthetic-no-force-push. Rulebook ID synthetic-book',
    'Rule ID synthetic-unscoped. Rulebook ID unscoped',
    'Rule ID synthetic-odd-mode. Rulebook ID synthetic-book',
    'Rule ID <b>synthetic-markup</b>. Rulebook ID synthetic-book',
  ]);
  expect(groups[1]!.querySelector('small > span[title]')?.textContent).toMatch(
    /^unscoped · latest /,
  );
  expect(groups[0]!.querySelector('small > span[title]')?.textContent).toMatch(/^latest /);
  for (const row of groups) expect(shownText(row)).not.toMatch(/synthetic-/);
  expect(within(root).getByText(NUMBERING)).toBeTruthy();
  expect(
    within(groups[0]!)
      .getAllByRole('cell')
      .slice(1)
      .map((cell) => cell.textContent),
  ).toEqual(['2', '1', '0', '0']);
  expect(within(root).getByText('6 rows · newest first')).toBeTruthy();
  expect(within(root).getByText('4 observed · latest first')).toBeTruthy();

  // Timeline: time, rule and version, recorded mode, optional context. Each
  // row names its exact pair's returned group.
  const fires = dataRows('Recorded timeline');
  expect(fires).toHaveLength(6);
  expect(fires.map(ruleTitle)).toEqual([
    'Observed group 1',
    'Observed group 1',
    'Observed group 2',
    'Observed group 1',
    'Observed group 3',
    'Observed group 4',
  ]);
  expect(identityOf(fires[2]!)).toBe(
    'Rule ID synthetic-unscoped. Rulebook ID unscoped. Fire ID fire-2. Recorded version draft (label)',
  );
  expect(identityOf(fires[4]!)).toMatch(/Recorded version none recorded$/);
  expect(fires[0]!.querySelector('time')?.getAttribute('dateTime')).toBe(
    synthetic.loaded().latest_fires[0]!.fired_at,
  );
  expect(within(fires[0]!).getByText('version 1')).toBeTruthy();
  expect(within(fires[0]!).getByText('claude · Bash · pre_tool_use')).toBeTruthy();
  expect(within(fires[1]!).getByText('gate')).toBeTruthy();
  expect(within(fires[2]!).getByText('version draft')).toBeTruthy();
  expect(within(fires[2]!).getByText('none recorded')).toBeTruthy();
  expect(within(fires[4]!).getByText('unrecognized')).toBeTruthy();
  expect(within(fires[4]!).getByText('no version recorded')).toBeTruthy();
  // Recorded values stay inert text.
  expect(root.querySelector('.xt-data-row b, .xt-data-row i')).toBeNull();
  expect(identityOf(fires[5]!)).toMatch(/^Rule ID <b>synthetic-markup<\/b>\. /);
  // Neither truncation is claimed.
  expect(root.querySelector('.xt-rulebook-truncated')).toBeNull();
  expectReadOnly(root);
  expectIdle();
  expect(controls.cancel).not.toHaveBeenCalled();
});

it('marks a lower bound with ≥ and says why from coverage and quality alone', async () => {
  await mount('/rulebook', sourceWith(answering(synthetic.lowerBound())));
  const root = page();
  await waitFor(() => expect(tileValue(0)).toBe('≥ 4'));
  expect(tiles()[0]!.querySelector('.xt-stat-aside')?.textContent).toBe('14d · lower bound');
  expect(status().textContent).toBe('Read complete · ≥ 4 recorded fires in ≥ 4 observed groups');
  const note = root.querySelector('.xt-rulebook-note')?.textContent;
  expect(note).toContain(
    'Counts are lower bounds (≥): an unfinished last line was skipped; 2 malformed lines were rejected; 1 fire ID was recorded with differing details and left out.',
  );
  expect(note).toMatch(/The source grew after it was captured; newer rows are not included\.$/);
  expect(note).not.toMatch(/exact/);
  expect(within(root).getByText('≥ 6 rows · newest first')).toBeTruthy();
  // Every group's count is a lower bound too, zero included.
  expect(
    within(dataRows('Observed rule groups')[0]!)
      .getAllByRole('cell')
      .slice(1)
      .map((cell) => cell.textContent),
  ).toEqual(['≥ 2', '≥ 1', '≥ 0', '≥ 0']);
  // The oldest observed row is never presented as where coverage starts.
  expect(root.textContent).not.toContain(synthetic.lowerBound().coverage.oldest_observed);
});

it('tells an exact empty snapshot from a lower-bound one that found nothing', async () => {
  const exact = await mount('/rulebook', sourceWith(answering(synthetic.emptyExact())));
  await waitFor(() => expect(tileValue(0)).toBe('0'));
  expect(within(page()).getAllByText('0 recorded rows in this source snapshot')).toHaveLength(2);
  expect(within(page()).queryByText('No rows found in the portion read')).toBeNull();
  exact.unmount();

  await mount('/rulebook', sourceWith(answering(synthetic.emptyLowerBound())));
  await waitFor(() => expect(tileValue(0)).toBe('≥ 0'));
  expect(within(page()).getAllByText('No rows found in the portion read')).toHaveLength(2);
  expect(within(page()).queryByText('0 recorded rows in this source snapshot')).toBeNull();
  expect(page().querySelector('.xt-rulebook-note')?.textContent).toMatch(
    /Counts are lower bounds \(≥\): 1 malformed line was rejected\./,
  );
});

it('lists the returned 100 groups and fires and flags each truncation apart', async () => {
  await mount('/rulebook', sourceWith(answering(synthetic.truncated())));
  const root = page();
  await waitFor(() => expect(tileValue(0)).toBe('220'));
  const groups = dataRows('Observed rule groups');
  const fires = dataRows('Recorded timeline');
  expect(groups).toHaveLength(100);
  expect(fires).toHaveLength(100);
  // Each listed fire's rule is a listed group; the first also has older rows.
  expect(fires.map(ruleTitle)).toEqual(groups.map(ruleTitle));
  expect(fires.map(identityOf).map((ids) => ids?.split('. Fire ID')[0])).toEqual(
    groups.map(identityOf),
  );
  expect(
    within(groups[0]!)
      .getAllByRole('cell')
      .slice(1)
      .map((cell) => cell.textContent),
  ).toEqual(['81', '0', '5', '0']);
  const flags = root.querySelectorAll('.xt-rulebook-truncated');
  expect([...flags].map((flag) => flag.textContent)).toEqual([
    'Showing 100 of 140 observed groups; every group is counted above.',
    'Showing the latest 100 of 225 recorded rows in the window.',
  ]);
  expect(within(root).getByText('140 observed · latest first')).toBeTruthy();
  // Only the status is announced, never the rows.
  expect(within(root).getAllByRole('status')).toHaveLength(1);
  expect(status().textContent).toBe('Read complete · 220 recorded fires in 140 observed groups');
  expectReadOnly(root);
});

it('shows every state without a snapshot as static copy, with a manual Refresh and no counts', async () => {
  const cases: [RuleActivityResult | 'rejected', string, string, RegExp][] = [
    [synthetic.unavailable, 'unavailable', 'Source unavailable', /Its ledger was not found\./],
    [
      synthetic.unsupportedSchema,
      'unavailable',
      'Source unavailable',
      /Its schema marker names a schema this app does not read \(schema 9\)\./,
    ],
    [
      synthetic.sourceChanged,
      'changed',
      'The source changed during the read',
      /It was shrunk while being read/,
    ],
    [synthetic.deadline, 'timed out', 'The read ran out of time', /No partial counts/],
    [synthetic.interruptedCancelled, 'cancelled', 'Read cancelled', /Nothing it may have counted/],
    [synthetic.busy, 'busy', 'Another rule activity read is still running', /Refresh once/],
    [synthetic.closed, 'closed', 'The rule activity reader has closed', /Nothing was read/],
    [synthetic.failed, 'failed', 'Rule activity could not be read', /before a snapshot/],
    [synthetic.invalidReadId, 'refused', 'The read was refused', /Nothing was read/],
    ['rejected', 'failed', 'Rule activity could not be read', /did not return an answer/],
  ];
  for (const [result, pill, lead, detail] of cases) {
    const controls = answering(
      result === 'rejected' ? () => Promise.reject(new Error('synthetic')) : result,
    );
    const view = await mount('/rulebook', sourceWith(controls));
    const root = page();
    await waitFor(() => expect(status().textContent).toBe(lead));
    expect(root.querySelector('.xt-rulebook-read .xt-state')?.textContent).toBe(pill);
    expect(root.querySelector('.xt-rulebook-read .xt-rulebook-detail')?.textContent).toMatch(
      detail,
    );
    expectNoCounts(root);
    expectFiresTile(lead, pill);
    expectReadOnly(root);
    expectIdle();
    // Nothing retries on its own.
    await settled();
    expect(controls.read).toHaveBeenCalledTimes(1);
    view.unmount();
  }
});

it('reads once per activation, refreshes only after the last read ended, and drops the old snapshot', async () => {
  const { controls, reads, cancels } = controlled();
  await mount('/rulebook', sourceWith(controls));
  const root = page();
  await waitFor(() => expect(reads).toHaveLength(1));
  expect(status().textContent).toBe('Reading the default local rulebook source…');
  expect(root.querySelector('.xt-rulebook-read .xt-state')?.textContent).toBe('reading');
  expectReading();
  expectNoCounts(root);
  expectFiresTile('Reading the default local rulebook source…', 'reading');
  expect(within(root).getAllByText('No rows are listed until a read completes.')).toHaveLength(2);
  // Refresh while reading asks nothing.
  fireEvent.click(refreshButton());
  await settled();
  expect(reads).toHaveLength(1);

  reads[0]!.resolve(synthetic.loaded());
  await waitFor(() => expect(tileValue(0)).toBe('4'));
  expect(tiles()[0]!.querySelector('.xt-unmeasured')).toBeNull();
  expect(tiles()[0]!.querySelector('.xt-stat-aside')?.textContent).toBe('14d · exact');
  expectIdle();
  // Cancel with nothing running asks nothing.
  fireEvent.click(cancelButton());
  expect(cancels).toEqual([]);

  // A double click is one refresh, and never a cancel of it.
  fireEvent.click(refreshButton());
  fireEvent.click(refreshButton());
  await waitFor(() => expect(reads).toHaveLength(2));
  await settled();
  expect(reads).toHaveLength(2);
  expect(reads[1]!.id).not.toBe(reads[0]!.id);
  // The earlier snapshot is gone while the new read runs.
  expectNoCounts(root);
  expectFiresTile('Reading the default local rulebook source…', 'reading');
  expectReading();

  reads[1]!.resolve(synthetic.busy);
  await waitFor(() =>
    expect(status().textContent).toBe('Another rule activity read is still running'),
  );
  expectNoCounts(root);
  await settled();
  expect(reads).toHaveLength(2);
  expect(cancels).toEqual([]);
});

it('stops a read on request, keeping focus in place, and shows nothing it counted', async () => {
  const { controls, reads, cancels } = controlled();
  await mount('/rulebook', sourceWith(controls));
  await waitFor(() => expect(reads).toHaveLength(1));
  const cancel = cancelButton();
  cancel.focus();
  fireEvent.click(cancel);
  expect(cancel.textContent).toBe('Stopping…');
  expect(offered(cancel)).toBe(false);
  expect(offered(refreshButton())).toBe(false);
  expect(status().textContent).toBe('Stopping the read…');
  expect(page().querySelector('.xt-rulebook-read .xt-state')?.textContent).toBe('stopping');
  expectFiresTile('Stopping the read…', 'stopping');
  expect(cancels).toEqual([reads[0]!.id]);
  // Pressing it again while stopping asks nothing more.
  fireEvent.click(cancel);
  expect(cancels).toHaveLength(1);
  // Even a snapshot that completed first is not shown once Cancel was asked.
  reads[0]!.resolve(synthetic.loaded());
  await waitFor(() => expect(status().textContent).toBe('Read cancelled'));
  expectNoCounts(page());
  expectFiresTile('Read cancelled', 'cancelled');
  expectIdle();
  expect(cancelButton()).toBe(cancel);
  expect(document.activeElement).toBe(cancel);
  const refresh = refreshButton();
  refresh.focus();
  fireEvent.click(refresh);
  await waitFor(() => expect(reads).toHaveLength(2));
  expect(document.activeElement).toBe(refresh);
  expect(cancels).toHaveLength(1);
});

it('cancels a read on leaving its view and ignores its late answer', async () => {
  const { controls, reads, cancels } = controlled();
  await mount('/rulebook/fires', sourceWith(controls));
  expect(screen.getByRole('heading', { level: 1, name: 'Recorded rule activity' })).toBeTruthy();
  await waitFor(() => expect(reads).toHaveLength(1));
  // The fires view is its own activation: leaving it for the overview cancels
  // its read and starts the overview's.
  fireEvent.click(within(page()).getByRole('link', { name: '← Rulebook' }));
  expect(screen.getByRole('heading', { level: 1, name: 'Rulebook' })).toBeTruthy();
  expect(cancels).toEqual([reads[0]!.id]);
  await waitFor(() => expect(reads).toHaveLength(2));
  reads[0]!.resolve(synthetic.loaded());
  await settled();
  expect(status().textContent).toBe('Reading the default local rulebook source…');
  expectNoCounts(page());
  // The native side may still be finishing the first: busy, retried by hand only.
  reads[1]!.resolve(synthetic.busy);
  await waitFor(() =>
    expect(status().textContent).toBe('Another rule activity read is still running'),
  );
  await settled();
  expect(reads).toHaveLength(2);
  fireEvent.click(refreshButton());
  await waitFor(() => expect(reads).toHaveLength(3));
  expect(cancels).toEqual([reads[0]!.id]);
});

it('shows the fires address as the same bounded timeline with a way back', async () => {
  await mount('/rulebook/fires', sourceWith(answering(synthetic.truncated())));
  const root = page();
  expect(crumb().getByText('fires')).toBeTruthy();
  expect(screen.getByRole('button', { name: 'Rulebook' }).getAttribute('aria-current')).toBe(
    'page',
  );
  await waitFor(() => expect(dataRows('Recorded timeline')).toHaveLength(100));
  expect(
    within(root)
      .getAllByRole('heading', { level: 2 })
      .map((heading) => heading.textContent),
  ).toEqual(['Recorded timeline']);
  expect(root.querySelector('.xt-rulebook-tiles')).toBeNull();
  expect(root.querySelector('.xt-rulebook-source')?.textContent).toMatch(
    /window 2026-09-09T09:00:00Z to 2026-09-23T09:00:00Z/,
  );
  expect(root.querySelector('.xt-rulebook-truncated')?.textContent).toBe(
    'Showing the latest 100 of 225 recorded rows in the window.',
  );
  // Without the groups panel, the numbering is explained under the timeline.
  expect(root.querySelector('.xt-section-footer')?.textContent).toBe(
    `Showing the latest 100 of 225 recorded rows in the window.${NUMBERING}`,
  );
  expectReadOnly(root);
});

it('makes no read in a window without a source', async () => {
  await mount('/rulebook', sourceWith(undefined));
  const root = page();
  expect(status().textContent).toBe('Recorded rule activity is read only in the desktop app');
  const bar = root.querySelector<HTMLElement>('.xt-rulebook-read')!;
  expect(within(bar).queryAllByRole('button')).toHaveLength(0);
  expect(within(root).getAllByText('No rows are listed in this preview.')).toHaveLength(2);
  expectNoCounts(root);
  expectFiresTile('Recorded rule activity is read only in the desktop app', 'unavailable');
});

it('repeats a rule address as asked for without presenting it as a rule or reading', async () => {
  const controls = answering(synthetic.loaded());
  await mount('/rulebook/not-a-real-rule', sourceWith(controls));
  const root = page();
  expect(screen.getByRole('heading', { level: 1, name: 'Rule detail' })).toBeTruthy();
  expect(screen.getByText('This rule cannot be shown')).toBeTruthy();
  expect(screen.getByText(/not evidence that the rule exists/)).toBeTruthy();
  expect(screen.getByText('/rulebook/not-a-real-rule')).toBeTruthy();
  // The chrome names the kind of page, not the unmatched identifier.
  expect(crumb().getByText('rule')).toBeTruthy();
  expect(crumb().queryByText('not-a-real-rule')).toBeNull();
  expect(
    within(root)
      .getAllByRole('link')
      .map((link) => link.getAttribute('href')),
  ).toEqual(['/rulebook']);
  expect(within(root).queryAllByRole('button')).toHaveLength(0);
  expect(within(root).queryAllByRole('row')).toHaveLength(0);
  await settled();
  expect(controls.read).not.toHaveBeenCalled();
  expect(controls.cancel).not.toHaveBeenCalled();
});

it('keeps an unusual address as inert text', async () => {
  await mount('/rulebook/%3Cb%3Ex%3C%2Fb%3E%20%E2%9C%93');
  expect(screen.getByRole('heading', { level: 1, name: 'Rule detail' })).toBeTruthy();
  const address = page().querySelector('.xt-rulebook-address span');
  expect(address?.textContent).toBe('/rulebook/%3Cb%3Ex%3C%2Fb%3E%20%E2%9C%93');
  expect(address?.querySelector('b')).toBeNull();
  await act(async () => {});
});

it('opens a tile’s definition from the keyboard', async () => {
  await mount('/rulebook', sourceWith(answering(synthetic.loaded())));
  await waitFor(() => expect(tileValue(0)).toBe('4'));
  act(() => tiles()[1]!.focus());
  expect(await screen.findByRole('tooltip')).toBeTruthy();
  expect(screen.getByRole('tooltip').textContent).toMatch(/not whether an agent was stopped/);
  await act(async () => {});
});

/** Each listed group's IDs with the name and counts its row shows. */
const groupsById = (): Record<string, string[]> =>
  Object.fromEntries(
    dataRows('Observed rule groups').map((row) => [
      identityOf(row),
      [
        ruleTitle(row),
        ...within(row)
          .getAllByRole('cell')
          .slice(1)
          .map((cell) => cell.textContent),
      ],
    ]),
  );
const tooltip = () => screen.queryByRole('tooltip');

it('names each group by its snapshot position and keeps its full IDs one hover or focus away', async () => {
  const answer = synthetic.identities();
  await mount('/rulebook', sourceWith(answering(answer)));
  const root = page();
  await waitFor(() => expect(tileValue(0)).toBe('252'));
  const [rule0, rule1, rule2] = synthetic.IDENTITY_PAIRS.map((pair) => pair.rule_id);
  const groups = dataRows('Observed rule groups');
  expect(groups.map(ruleTitle)).toEqual(
    Array.from({ length: 8 }, (_, index) => `Observed group ${index + 1}`),
  );
  // One rule ID recorded in a rulebook and unscoped is two groups, apart.
  expect(identityOf(groups[0]!)).toBe(`Rule ID ${rule0}. Rulebook ID ${synthetic.BOOK_A}`);
  expect(identityOf(groups[7]!)).toBe(`Rule ID ${rule0}. Rulebook ID unscoped`);
  expect(groups[7]!.querySelector('small > span[title]')?.textContent).toMatch(
    /^unscoped · latest /,
  );
  // Counts stay the snapshot's own, per exact pair.
  expect(groupsById()[`Rule ID ${rule0}. Rulebook ID ${synthetic.BOOK_A}`]).toEqual([
    'Observed group 1',
    '25',
    '7',
    '0',
    '0',
  ]);

  // Each timeline row names its pair's returned group, in the returned order.
  const fires = dataRows('Recorded timeline');
  expect(fires).toHaveLength(100);
  expect(fires.map(ruleTitle)).toEqual(
    answer.latest_fires.map((_, index) => `Observed group ${(index % 8) + 1}`),
  );
  expect(identityOf(fires[9]!)).toBe(
    `Rule ID ${rule1}. Rulebook ID ${synthetic.BOOK_A}. Fire ID ${answer.latest_fires[9]!.fire_id}. Recorded version 2`,
  );
  expect(within(fires[9]!).getByText('version 2')).toBeTruthy();
  // No raw ID reaches a row's visible text; each control's name is its own.
  for (const row of [...groups, ...fires]) expect(shownText(row)).not.toMatch(/[abf]{8}-0000/);
  const controls = [...root.querySelectorAll<HTMLElement>('.xt-rulebook-identity')];
  expect(controls).toHaveLength(108);
  expect(new Set(controls.map((control) => control.getAttribute('aria-label'))).size).toBe(108);

  // The control is a named, described button, one tab stop in its row.
  const third = within(table('Observed rule groups')).getByRole('button', {
    name: 'ID of observed group 3',
    description: `Rule ID ${rule2}. Rulebook ID ${synthetic.BOOK_A}`,
  });
  expect(third.tagName).toBe('BUTTON');
  expect(third.textContent).toBe('ID');
  expect(
    within(table('Recorded timeline')).getByRole('button', {
      name: 'ID of timeline row 10, observed group 2',
    }),
  ).toBe(fires[9]!.querySelector('.xt-rulebook-identity'));

  // Focus shows the full IDs over the page, outside the list; Escape closes
  // them and leaves focus where it was.
  act(() => third.focus());
  const shown = await screen.findByRole('tooltip');
  expect(shown.textContent).toBe(
    `Observed group 3 is this group’s position in this snapshot, not a rule name.Rule ID${rule2}Rulebook ID${synthetic.BOOK_A}`,
  );
  expect(root.contains(shown)).toBe(false);
  fireEvent.keyDown(third, { key: 'Escape' });
  await waitFor(() => expect(tooltip()).toBeNull());
  expect(document.activeElement).toBe(third);
  act(() => third.blur());

  // So does a pointer, for a timeline row's fire and version too.
  const row10 = fires[9]!.querySelector<HTMLElement>('.xt-rulebook-identity')!;
  fireEvent.pointerEnter(row10, { pointerType: 'mouse' });
  fireEvent.mouseEnter(row10);
  fireEvent.mouseMove(row10);
  expect((await screen.findByRole('tooltip')).textContent).toContain(
    `Fire ID${answer.latest_fires[9]!.fire_id}Recorded version2`,
  );
  fireEvent.pointerLeave(row10, { pointerType: 'mouse' });
  fireEvent.mouseLeave(row10);
  await waitFor(() => expect(tooltip()).toBeNull());
  expectReadOnly(root);
});

it('leaves a listed fire whose group is past the returned 100 unnumbered', async () => {
  await mount('/rulebook', sourceWith(answering(synthetic.pastCap())));
  await waitFor(() => expect(tileValue(0)).toBe('220'));
  const groups = dataRows('Observed rule groups');
  const fires = dataRows('Recorded timeline');
  expect(identityOf(groups[99]!)).toBe('Rule ID synthetic-rule-100. Rulebook ID synthetic-book');
  expect(groups.map(identityOf)).not.toContain(
    'Rule ID synthetic-rule-99. Rulebook ID synthetic-book',
  );
  // No number is guessed for it; its IDs are still one focus away.
  expect(fires.map(ruleTitle).slice(98)).toEqual(['Observed group 99', 'Recorded rule']);
  const last = within(fires[99]!).getByRole('button', {
    name: 'ID of timeline row 100, recorded rule',
    description:
      'Rule ID synthetic-rule-99. Rulebook ID synthetic-book. Fire ID fire-99. Recorded version 2',
  });
  act(() => last.focus());
  expect((await screen.findByRole('tooltip')).textContent).toMatch(
    /^This rule’s group is not among the 100 returned, so it has no position here\./,
  );
  act(() => last.blur());
  await waitFor(() => expect(tooltip()).toBeNull());
});

it('renumbers groups on Refresh without changing any group’s IDs or counts', async () => {
  const { controls, reads } = controlled();
  await mount('/rulebook', sourceWith(controls));
  await waitFor(() => expect(reads).toHaveLength(1));
  reads[0]!.resolve(synthetic.identities());
  await waitFor(() => expect(tileValue(0)).toBe('252'));
  const before = groupsById();
  fireEvent.click(refreshButton());
  await waitFor(() => expect(reads).toHaveLength(2));
  reads[1]!.resolve(synthetic.identitiesRefreshed());
  await waitFor(() => expect(tileValue(0)).toBe('253'));
  const after = groupsById();
  expect(Object.keys(after).sort()).toEqual(Object.keys(before).sort());
  // The pair with a newer row is now first, with that row counted...
  const [rule0] = synthetic.IDENTITY_PAIRS.map((pair) => pair.rule_id);
  const newer = `Rule ID ${rule0}. Rulebook ID unscoped`;
  expect(before[newer]![0]).toBe('Observed group 8');
  expect(after[newer]).toEqual([
    'Observed group 1',
    String(Number(before[newer]![1]) + 1),
    ...before[newer]!.slice(2),
  ]);
  // ...and every other pair is one place later, its counts as they were.
  for (const [ids, [name, ...counts]] of Object.entries(before)) {
    if (ids === newer) continue;
    expect(after[ids]).toEqual([`Observed group ${Number(name!.split(' ')[2]) + 1}`, ...counts]);
  }
  expect(reads).toHaveLength(2);
});
