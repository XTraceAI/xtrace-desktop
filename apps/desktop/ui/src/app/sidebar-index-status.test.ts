import { describe, expect, it } from 'vitest';
import type { NativeHostState } from '../data/generated/NativeHostState';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import {
  pluginReceiver,
  sidebarIndexFacts,
  sidebarIndexStatus,
  type SidebarIndexFacts,
} from './sidebar-index-status';

// Synthetic statuses only: no local history, registry or receipt is read.
const host = (name: string, state: NativeHostState, detail: string | null = null) => ({
  host: name,
  state,
  detail,
});
const facts = (
  phase: SidebarIndexFacts['phase'],
  freshness: SidebarIndexFacts['freshness'],
  hosts: SidebarIndexFacts['hosts'] = [host('claude', 'complete')],
): SidebarIndexFacts => ({ phase, freshness, hosts });
const ready = { phase: 'ready' } as const;
const live = { freshness: 'live' } as const;
const heard = { readFailed: false, live: 'connected', caughtUp: true } as const;

describe('the state the compact trigger shows', () => {
  const cases: [string, SidebarIndexFacts, string, string, 'live' | 'attention' | 'idle'][] = [
    [
      'scanning',
      facts({ phase: 'scanning' }, { freshness: 'unknown' }),
      'scanning',
      'Scanning',
      'idle',
    ],
    ['ready and watched', facts(ready, live), 'updating', 'Updating', 'live'],
    [
      'ready, watching not established',
      facts(ready, { freshness: 'unknown' }),
      'ready',
      'Ready',
      'idle',
    ],
    [
      'degraded watcher',
      facts(ready, { freshness: 'degraded', reason: 'watcher lost' }),
      'degraded',
      'Updates interrupted',
      'attention',
    ],
    ['stopped', facts({ phase: 'stopped' }, live), 'stopped', 'Stopped', 'attention'],
    [
      'disabled',
      facts({ phase: 'disabled', reason: 'the index database could not be opened' }, live, []),
      'disabled',
      'Disabled',
      'attention',
    ],
  ];
  it.each(cases)('%s', (_name, status, label, title, tone) => {
    const shown = sidebarIndexStatus({ status, ...heard });
    expect([shown.label, shown.title, shown.tone]).toEqual([label, title, tone]);
    expect(shown.notes).toBeUndefined();
  });

  it('says checking before the first read answers, and unknown when that read fails', () => {
    expect(sidebarIndexStatus({ status: undefined, ...heard })).toMatchObject({
      label: 'checking',
      tone: 'idle',
      hosts: [],
    });
    expect(
      sidebarIndexStatus({
        status: undefined,
        readFailed: true,
        caughtUp: true,
        live: 'connected',
      }),
    ).toMatchObject({
      label: 'unknown',
      title: 'Status unavailable',
      tone: 'attention',
      hosts: [],
    });
  });

  it('keeps every state word within the width of the status row', () => {
    // 17 characters of 10.5px Geist Mono fit beside the two footer controls.
    const labels = [
      ...cases.map(([, status]) => sidebarIndexStatus({ status, ...heard }).label),
      ...cases.map(
        ([, status]) =>
          sidebarIndexStatus({ status, readFailed: false, caughtUp: true, live: 'failed' }).label,
      ),
      'partial?',
      'checking',
      'unknown',
    ];
    for (const label of labels) expect(`index · ${label}`.length).toBeLessThanOrEqual(17);
  });

  it('reports the reason a disabled or degraded index gives, without doubling its full stop', () => {
    expect(
      sidebarIndexStatus({
        status: facts(
          { phase: 'disabled', reason: 'fixture mode uses a disposable database' },
          live,
        ),
        ...heard,
      }).summary,
    ).toBe('Local history is not being indexed: fixture mode uses a disposable database.');
    expect(
      sidebarIndexStatus({
        status: facts(ready, { freshness: 'degraded', reason: 'watcher lost.' }),
        ...heard,
      }).summary,
    ).toBe('Watching for changes is degraded: watcher lost. The index may be out of date.');
  });
});

describe('edges of the reported status', () => {
  it('ends the sentence cleanly when a disabled or degraded index gives no reason', () => {
    expect(
      sidebarIndexStatus({ status: facts({ phase: 'disabled', reason: '  ' }, live), ...heard })
        .summary,
    ).toBe('Local history is not being indexed.');
    expect(
      sidebarIndexStatus({
        status: facts(ready, { freshness: 'degraded', reason: '' }),
        ...heard,
      }).summary,
    ).toBe('Watching for changes is degraded. The index may be out of date.');
  });

  it('never words a phase this build does not know as ready', () => {
    // The generated union is closed; this is the failure direction if it ever is not.
    const unrecognised = { phase: 'rebuilding' } as unknown as SidebarIndexFacts['phase'];
    for (const connection of ['connected', 'failed'] as const)
      expect(
        sidebarIndexStatus({
          status: facts(unrecognised, live),
          readFailed: false,
          caughtUp: true,
          live: connection,
        }),
      ).toMatchObject({ label: 'unknown', title: 'Status unavailable', tone: 'attention' });
  });

  it('keeps an unrecognised freshness off the green dot', () => {
    const unrecognised = { freshness: 'catching-up' } as unknown as SidebarIndexFacts['freshness'];
    expect(sidebarIndexStatus({ status: facts(ready, unrecognised), ...heard })).toMatchObject({
      label: 'ready',
      tone: 'idle',
    });
  });
});

describe('host scans qualify a ready index', () => {
  it('never lets a live watcher hide a partial or failed host scan', () => {
    const shown = sidebarIndexStatus({
      status: facts(ready, live, [
        host('claude', 'incomplete'),
        host('codex', 'reader_failed', 'reader exited with status 1'),
        host('cursor', 'complete'),
      ]),
      ...heard,
    });
    expect(shown).toMatchObject({
      label: 'partial',
      title: 'Updating · partial',
      tone: 'attention',
    });
    expect(shown.notes).toEqual([
      'Last scan not complete for claude (incomplete), codex (reader failed). The index may be incomplete or out of date for those hosts.',
    ]);
    expect(shown.hosts).toEqual([
      { host: 'claude', state: 'Incomplete', attention: true },
      {
        host: 'codex',
        state: 'Reader failed',
        attention: true,
        reason: 'reader exited with status 1',
      },
      { host: 'cursor', state: 'Complete' },
    ]);
  });

  it.each([
    ['missing_runtime', 'Python runtime missing'],
    ['pin_mismatch', 'Reader pin mismatch'],
    ['cancelled', 'Cancelled'],
    ['pending', 'Not scanned yet'],
  ] as const)('qualifies a ready index whose host is %s', (state, words) => {
    const shown = sidebarIndexStatus({
      status: facts(ready, { freshness: 'unknown' }, [host('codex', state)]),
      ...heard,
    });
    expect(shown).toMatchObject({ label: 'partial', title: 'Ready · partial', tone: 'attention' });
    expect(shown.hosts).toEqual([{ host: 'codex', state: words, attention: true }]);
  });

  it('shows an absent source as its own neutral fact, not a reader failure', () => {
    const shown = sidebarIndexStatus({
      status: facts(ready, live, [host('claude', 'complete'), host('cursor', 'missing_source')]),
      ...heard,
    });
    expect(shown).toMatchObject({ label: 'updating', tone: 'live' });
    expect(shown.notes).toBeUndefined();
    expect(shown.hosts).toEqual([
      { host: 'claude', state: 'Complete' },
      { host: 'cursor', state: 'No local history found' },
    ]);
  });

  it('treats hosts the initial scan has not reached as waiting, not as gaps', () => {
    const shown = sidebarIndexStatus({
      status: facts({ phase: 'scanning' }, { freshness: 'unknown' }, [
        host('claude', 'pending'),
        host('codex', 'pending'),
      ]),
      ...heard,
    });
    expect(shown).toMatchObject({ label: 'scanning', tone: 'idle' });
    expect(shown.notes).toBeUndefined();
    expect(shown.hosts.every((row) => row.attention === undefined)).toBe(true);
  });

  it('says a failed scan may leave the index short, never that stored history is missing', () => {
    // History indexed by earlier scans is still there when the last one fails.
    const shown = sidebarIndexStatus({
      status: facts(ready, live, [host('codex', 'reader_failed')]),
      ...heard,
    });
    expect(shown.notes).toEqual([
      'Last scan not complete for codex (reader failed). The index may be incomplete or out of date for that host.',
    ]);
    expect(JSON.stringify(shown)).not.toMatch(/is missing|are missing|was lost|not indexed/i);
  });

  it('keeps the watcher problem on the trigger and still names the host gaps', () => {
    const shown = sidebarIndexStatus({
      status: facts(ready, { freshness: 'degraded', reason: 'watcher lost' }, [
        host('claude', 'incomplete'),
      ]),
      ...heard,
    });
    expect(shown.label).toBe('degraded');
    expect(shown.notes?.[0]).toContain('claude (incomplete)');
  });
});

describe('a cached status while this renderer does not hear the index', () => {
  const cached = facts(ready, live);

  it('is last known, never live, while live updates have failed', () => {
    const shown = sidebarIndexStatus({
      status: cached,
      readFailed: false,
      caughtUp: true,
      live: 'failed',
    });
    expect(shown).toMatchObject({
      label: 'updating?',
      title: 'Last known: Updating',
      tone: 'attention',
    });
    expect(shown.notes).toEqual([
      'Live updates are unavailable, so this is the last status this window read and it may be out of date. Use Reconnect.',
    ]);
  });

  it('stays last known while a reconnect is under way', () => {
    const shown = sidebarIndexStatus({
      status: cached,
      readFailed: false,
      caughtUp: true,
      live: 'connecting',
    });
    expect(shown).toMatchObject({ label: 'updating?', tone: 'attention' });
    expect(shown.notes?.[0]).toMatch(/^Live updates are reconnecting/);
  });

  it('is last known when the latest read failed over a cached status', () => {
    const shown = sidebarIndexStatus({
      status: cached,
      readFailed: true,
      caughtUp: true,
      live: 'connected',
    });
    expect(shown).toMatchObject({ label: 'updating?', title: 'Last known: Updating' });
    expect(shown.notes?.[0]).toMatch(/^The latest status read failed/);
  });

  it('keeps the host qualification beside the staleness one', () => {
    const shown = sidebarIndexStatus({
      status: facts(ready, live, [host('codex', 'reader_failed')]),
      readFailed: false,
      caughtUp: true,
      live: 'failed',
    });
    expect(shown.label).toBe('partial?');
    expect(shown.notes).toHaveLength(2);
  });

  it('stays last known after the listeners register, until a read that began since succeeds', () => {
    const shown = sidebarIndexStatus({
      status: cached,
      readFailed: false,
      live: 'connected',
      caughtUp: false,
    });
    expect(shown).toMatchObject({
      label: 'updating?',
      title: 'Last known: Updating',
      tone: 'attention',
    });
    expect(shown.notes).toEqual([
      'Live updates are back and the status is being read again, so this is the last status this window read and it may be out of date.',
    ]);
    // A catch-up read that failed is said to have failed, not to be under way.
    expect(
      sidebarIndexStatus({ status: cached, readFailed: true, live: 'connected', caughtUp: false })
        .notes?.[0],
    ).toMatch(/^The latest status read failed/);
    // Caught up, the same status is current again.
    expect(
      sidebarIndexStatus({ status: cached, readFailed: false, live: 'connected', caughtUp: true }),
    ).toMatchObject({ label: 'updating', tone: 'live' });
  });

  it('claims nothing extra while there is no status to qualify', () => {
    expect(
      sidebarIndexStatus({ status: undefined, readFailed: false, caughtUp: true, live: 'failed' }),
    ).toMatchObject({ label: 'checking', tone: 'idle' });
  });
});

describe('the plugin receiver', () => {
  it('is off, listening or unknown, and never carries a port', () => {
    expect(pluginReceiver(false)).toEqual({ status: 'off' });
    expect(pluginReceiver(true)).toEqual({ status: 'listening' });
    expect(pluginReceiver(undefined)).toEqual({ status: 'unknown' });
  });
});

it('infers nothing about installation, capture, surfaces or ports from any status', () => {
  const states: NativeHostState[] = [
    'pending',
    'complete',
    'incomplete',
    'missing_source',
    'missing_runtime',
    'pin_mismatch',
    'reader_failed',
    'cancelled',
  ];
  const phases: SidebarIndexFacts['phase'][] = [
    { phase: 'disabled', reason: 'r' },
    { phase: 'scanning' },
    ready,
    { phase: 'stopped' },
  ];
  const freshness: SidebarIndexFacts['freshness'][] = [
    { freshness: 'unknown' },
    live,
    { freshness: 'degraded', reason: 'r' },
  ];
  for (const phase of phases)
    for (const fresh of freshness)
      for (const state of states)
        for (const connection of ['connected', 'failed', 'connecting'] as const)
          for (const caughtUp of [true, false]) {
            const shown = sidebarIndexStatus({
              status: facts(phase, fresh, [host('codex', state)]),
              readFailed: false,
              caughtUp,
              live: connection,
            });
            const text = JSON.stringify(shown);
            expect(text).not.toMatch(/captur|install|surface|healthy|connected|:\d/i);
            // Only a live watcher with nothing to qualify draws the green dot.
            if (shown.tone === 'live') {
              expect([phase.phase, fresh.freshness, connection, caughtUp]).toEqual([
                'ready',
                'live',
                'connected',
                true,
              ]);
              expect(['complete', 'missing_source']).toContain(state);
            }
          }
});

it('narrows a full status to the facts the sidebar shows', () => {
  const status: NativeIndexStatus = {
    phase: { phase: 'scanning' },
    freshness: { freshness: 'unknown' },
    python: { state: 'available', path: '/synthetic/python3' },
    readers: { state: 'unavailable', reason: 'synthetic' },
    hosts: [
      {
        host: 'claude',
        state: 'pending',
        detail: null,
        sessions_imported: 4,
        sessions_partial: 1,
        sessions_skipped: 2,
        skipped_conversations: [
          { conversation_id: null, reason: 'unknown' },
          { conversation_id: null, reason: 'unknown' },
        ],
        skipped_conversations_omitted: 0,
        records_new: 9,
        records_enriched: 3,
        diagnostics: 1,
      },
    ],
    reconciles: 7,
    files_scanned: 41,
  };
  expect(sidebarIndexFacts(status)).toEqual({
    phase: { phase: 'scanning' },
    freshness: { freshness: 'unknown' },
    hosts: [{ host: 'claude', state: 'pending', detail: null }],
  });
});
