import { expect, it } from 'vitest';
import {
  formatHosts,
  listAddress,
  parseHosts,
  parseWithPrs,
  parseHost,
  parseRange,
  parseSearch,
  parseSessionSort,
  SEARCH_MAX,
  sessionHref,
  sessionParams,
} from './session-search';

it('carries the whole canonical ID, the reported host and the selected range', () => {
  const params = listAddress(
    { sessionId: '00000000-0000-4000-8000-000000000001', host: 'claude' },
    '30d',
  );
  // The whole ID, not the shortened one a row shows: a prefix could match a
  // different session, and this address promises only a list that holds it.
  expect(params.get(sessionParams.search)).toBe('00000000-0000-4000-8000-000000000001');
  expect(params.get(sessionParams.host)).toBe('claude');
  expect(params.get(sessionParams.range)).toBe('30d');
});

it('drops a host the filter cannot apply instead of failing the search', () => {
  const params = listAddress({ sessionId: 'session-1', host: 'unheard-of' }, '14d');
  expect(params.has(sessionParams.host)).toBe(false);
  expect(params.get(sessionParams.search)).toBe('session-1');
  expect(params.get(sessionParams.range)).toBe('14d');
});

it('opens one session by name, encoded as the path segment it is', () => {
  // A stored identity is host text, not a checked shape, so the address must
  // survive characters that carry meaning in a path or a query string.
  const sessionId = 'codex-a b&c=d#e/f+g?h';
  const href = sessionHref(sessionId, listAddress({ sessionId, host: 'codex' }, '7d'));
  expect(href).not.toContain(' ');
  // Everything after the route's own single separator is the identity: the
  // slash, hash and question mark inside it are encoded, not structural.
  const [path, query] = [href.slice(0, href.indexOf('?')), href.slice(href.indexOf('?'))];
  expect(decodeURIComponent(path.replace('/sessions/', ''))).toBe(sessionId);
  expect(path.split('/').length).toBe(3);
  const params = new URLSearchParams(query);
  expect(params.get(sessionParams.search)).toBe(sessionId);
  expect(params.get(sessionParams.host)).toBe('codex');
  expect(params.get(sessionParams.range)).toBe('7d');
});

it('opens a session with no list state beside it', () => {
  expect(sessionHref('session-1')).toBe('/sessions/session-1');
  expect(sessionHref('session-1', new URLSearchParams('pr=1'))).toBe('/sessions/session-1');
});

it('carries only the list state the Sessions page reads', () => {
  // An unrelated parameter is not this address's to forward: the page reads
  // three keys, and a link that carried more would restore something else.
  const carried = new URLSearchParams({ q: 'atlas', host: 'claude', range: '14d', pr: '7' });
  const href = sessionHref('session-1', carried);
  const params = new URLSearchParams(href.slice(href.indexOf('?')));
  expect([...params.keys()].sort()).toEqual(['host', 'q', 'range']);
});

it.each([
  ['claude', 'claude'],
  ['codex', 'codex'],
  ['cursor', 'cursor'],
  ['other', null],
  ['CLAUDE', null],
  ['', null],
  [null, null],
] as const)('reads host %s as %s', (value, expected) => {
  expect(parseHost(value)).toBe(expected);
});

it.each([
  ['7d', '7d'],
  ['14d', '14d'],
  ['30d', '30d'],
  ['31d', null],
  ['7', null],
  ['', null],
  [null, null],
] as const)('reads range %s as %s', (value, expected) => {
  expect(parseRange(value)).toBe(expected);
});

it('bounds the search to what the store accepts and treats an absent one as empty', () => {
  expect(parseSearch(null)).toBe('');
  expect(parseSearch('atlas')).toBe('atlas');
  expect(parseSearch('x'.repeat(SEARCH_MAX + 50))).toHaveLength(SEARCH_MAX);
});

it.each([
  // The original single-host address reads exactly as it always did.
  ['claude', ['claude']],
  ['codex,claude', ['claude', 'codex']],
  ['cursor,cursor', ['cursor']],
  ['claude,pretend', ['claude']],
  // Every offered host is no filter, so hosts the menu does not offer stay listed.
  ['claude,codex,cursor', null],
  ['pretend', null],
  ['', null],
  [null, null],
] as const)('reads host set %s as %s', (value, expected) => {
  expect(parseHosts(value)).toEqual(expected);
});

it('writes a host set canonically and drops it when it names every host', () => {
  expect(formatHosts(['codex', 'claude'])).toBe('claude,codex');
  expect(formatHosts(['cursor', 'codex', 'claude'])).toBe(null);
  expect(formatHosts([])).toBe(null);
});

it('reads the pull-request filter only from its explicit value', () => {
  expect(parseWithPrs('1')).toBe(true);
  for (const value of ['0', 'true', '', null]) expect(parseWithPrs(value)).toBe(false);
});

it('carries the host set and the pull-request filter back to the list', () => {
  const carried = new URLSearchParams({ host: 'claude,codex', with_prs: '1', pr: '7' });
  const href = sessionHref('session-1', carried);
  const params = new URLSearchParams(href.slice(href.indexOf('?')));
  expect(params.get(sessionParams.host)).toBe('claude,codex');
  expect(params.get(sessionParams.withPrs)).toBe('1');
  expect(params.has('pr')).toBe(false);
});

it('carries Recently active through the detail route and defaults unknown sort to Started', () => {
  expect(sessionHref('session-1', new URLSearchParams('sort=recently_active&range=14d'))).toBe(
    '/sessions/session-1?range=14d&sort=recently_active',
  );
  expect(parseSessionSort('recently_active')).toBe('recently_active');
  for (const value of [null, 'unknown', 'started']) expect(parseSessionSort(value)).toBe('started');
});
