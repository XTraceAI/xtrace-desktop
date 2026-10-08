import type { TimeRange } from '../kit/TopBar';

/**
 * The Sessions list's supported URL state, shared by the list itself and by
 * every link that opens it.
 *
 * Only these keys are read; anything else in the query string is left alone,
 * so an unrelated parameter (`pr`, say) still reaches the page untouched.
 */
export const sessionParams = {
  /** Substring matched against session ID, repository, branch and saved title. */
  search: 'q',
  /**
   * One host (`host=claude`, the original form every existing link uses) or a
   * comma-separated set (`host=claude,codex`). Absent means every host.
   */
  host: 'host',
  /** `1` lists only sessions with a recorded pull-request link. */
  withPrs: 'with_prs',
  /** The event window the list's measurements are taken over. */
  range: 'range',
  sort: 'sort',
} as const;

/** The hosts the Sessions filter offers; the store rejects anything else. */
export const sessionHosts = ['claude', 'codex', 'cursor'] as const;
const hosts = sessionHosts;
export type SessionHost = (typeof hosts)[number];
const ranges = ['7d', '14d', '30d'] as const;

/** The store rejects a longer filter, and the search box caps typing here. */
export const SEARCH_MAX = 256;

/**
 * A host the filter can actually apply, or nothing. An unrecognized host is
 * dropped rather than passed on: the select has no option for it, and the
 * store would reject the query, so the list would fail instead of filtering.
 */
export function parseHost(value: string | null | undefined): SessionHost | null {
  return hosts.find((host) => host === value) ?? null;
}

/**
 * The host set the list applies, in canonical order, or nothing for every
 * host. A single host is the legacy address and reads exactly as before;
 * unsupported members are dropped. A set naming every offered host is no
 * filter at all, so it also lists hosts the menu does not offer (`other`).
 */
export function parseHosts(value: string | null | undefined): SessionHost[] | null {
  const named = new Set((value ?? '').split(','));
  const selected = hosts.filter((host) => named.has(host));
  return selected.length === 0 || selected.length === hosts.length ? null : selected;
}

/** The address value for a host set, or nothing when it is every host. */
export function formatHosts(selected: readonly string[]): string | null {
  return parseHosts(selected.join(','))?.join(',') ?? null;
}

/** Whether the list shows only sessions with a recorded pull-request link. */
export function parseWithPrs(value: string | null | undefined): boolean {
  return value === '1';
}

/** A selectable range, or nothing when the value is absent or unsupported. */
export function parseRange(value: string | null | undefined): TimeRange | null {
  return ranges.find((range) => range === value) ?? null;
}

/** Unrecognized or absent ordering keeps the existing Started default. */
export function parseSessionSort(value: string | null | undefined) {
  return value === 'recently_active' ? 'recently_active' : 'started';
}

/** The search text the list will run, bounded to what the store accepts. */
export function parseSearch(value: string | null | undefined): string {
  return (value ?? '').slice(0, SEARCH_MAX);
}

/**
 * Exactly the list state the Sessions page reads — search, host set,
 * pull-request filter and range — out of any address, and nothing else. Both
 * the link into a session and that session's own Back link carry this, so
 * leaving and returning restores the same list.
 */
export function listState(carry?: URLSearchParams | null): URLSearchParams {
  const params = new URLSearchParams();
  for (const key of [
    sessionParams.search,
    sessionParams.host,
    sessionParams.withPrs,
    sessionParams.range,
    sessionParams.sort,
  ]) {
    const value = carry?.get(key);
    if (value) params.set(key, value);
  }
  return params;
}

/**
 * A link that opens one session.
 *
 * The whole canonical identifier is the route's own segment, so the page opens
 * that session and not something sharing its first characters. Whatever list
 * state the reader came from travels beside it — the search, the host filter,
 * the pull-request filter and the range — so the page's own Back link restores the list they left
 * rather than an unfiltered one, and the measurements it shows are taken over
 * the window the linking page was showing.
 */
export function sessionHref(sessionId: string, carry?: URLSearchParams | null) {
  const query = listState(carry).toString();
  // The identifier is a path segment, so it is encoded as one: a host's own
  // identifier may carry a slash, a question mark or a hash.
  return `/sessions/${encodeURIComponent(sessionId)}${query ? `?${query}` : ''}`;
}

/**
 * The list state that finds one session, for a page that has none of its own.
 *
 * The Dashboard shows sessions but does not filter a list, so a link from it
 * carries the filters that *would* show that session: the whole canonical ID
 * as the search, the reported host when the filter supports it, and the range
 * the linking page was measuring over. Handed to {@link sessionHref} it
 * becomes the detail page's own Back address, so leaving the Dashboard for a
 * session and then stepping back lands on a list that holds it rather than on
 * an unfiltered one.
 */
export function listAddress(
  session: { sessionId: string; host: string },
  range: TimeRange,
): URLSearchParams {
  const params = new URLSearchParams();
  params.set(sessionParams.search, session.sessionId);
  const host = parseHost(session.host);
  if (host) params.set(sessionParams.host, host);
  params.set(sessionParams.range, range);
  return params;
}
