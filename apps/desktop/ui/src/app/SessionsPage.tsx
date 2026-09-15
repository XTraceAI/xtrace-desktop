import { useEffect, useState } from 'react';
import { useInfiniteQuery } from '@tanstack/react-query';
import { Link, useSearchParams } from 'react-router';
import { useData } from '../data/DataProvider';
import { queryKeys } from '../data/query-client';
import type { SessionRow } from '../data/generated/SessionRow';
import { DataTable, type Column } from '../kit/DataTable';
import { HostGlyph } from '../kit/HostGlyph';
import { Search } from '../kit/Search';
import { Button } from '../kit/Button';
import { SectionCard } from '../kit/SectionCard';
import { useNativeIndexStatus } from './useAppInfo';
import '../styles/sessions.css';

const columns: Column<SessionRow>[] = [
  {
    key: 'host',
    header: 'Host',
    width: '56px',
    render: (row) => <HostGlyph host={row.host} size={20} />,
  },
  {
    key: 'session',
    header: 'Session · repository · branch',
    width: 'minmax(220px, 1fr)',
    render: (row) => (
      <div className="xt-session-name">
        <strong title={row.id}>Session {row.id.replace(/^(codex|cursor)-/, '').slice(0, 8)}</strong>
        <span title={row.repo ?? undefined}>
          {row.repo?.replaceAll('\\', '/').split('/').filter(Boolean).at(-1) ??
            'Unknown repository'}{' '}
          · {row.branch ?? 'Unknown branch'}
        </span>
      </div>
    ),
  },
  {
    key: 'model',
    header: 'Model',
    width: '170px',
    render: (row) => <span title={row.model ?? 'Model unavailable'}>{row.model ?? '—'}</span>,
  },
  {
    key: 'first',
    header: 'First recorded',
    width: '180px',
    render: (row) =>
      row.first_ts ? (
        <time dateTime={row.first_ts}>
          {new Date(row.first_ts).toLocaleString(undefined, {
            year: 'numeric',
            month: 'short',
            day: 'numeric',
            hour: '2-digit',
            minute: '2-digit',
          })}
        </time>
      ) : (
        '—'
      ),
  },
  {
    key: 'records',
    header: 'Records',
    width: '76px',
    align: 'right',
    render: (row) => row.record_count.toLocaleString(),
  },
  {
    key: 'conflict',
    header: 'Source',
    width: '94px',
    render: (row) =>
      row.has_conflict ? (
        <span title="Some source observations disagree">Conflict</span>
      ) : (
        'Indexed'
      ),
  },
];

export function SessionsPage() {
  const { source } = useData();
  const index = useNativeIndexStatus();
  const [params] = useSearchParams();
  const [input, setInput] = useState('');
  const [search, setSearch] = useState('');
  const [host, setHost] = useState<string | null>(null);
  useEffect(() => {
    const timer = setTimeout(() => setSearch(input), 200);
    return () => clearTimeout(timer);
  }, [input]);
  const query = useInfiniteQuery({
    queryKey: [...queryKeys.sessions, search, host],
    initialPageParam: null as string | null,
    queryFn: ({ pageParam }) => source.sessionsList(search, host, pageParam),
    getNextPageParam: (page) => page.next ?? undefined,
    enabled: source.kind !== 'preview' && !params.has('pr'),
  });
  const rows = query.data?.pages.flatMap((page) => page.rows) ?? [];
  const incomplete = index.data?.hosts.some((h) => h.state !== 'complete');
  const scanning =
    index.data?.phase.phase === 'scanning' || index.data?.hosts.some((h) => h.state === 'pending');
  const degraded = index.data?.freshness.freshness === 'degraded';
  const indexUnavailable =
    source.kind === 'native' &&
    (index.isError || ['disabled', 'stopped'].includes(index.data?.phase.phase ?? ''));

  const coverageNotice = indexUnavailable
    ? 'Indexing is unavailable; showing previously indexed history.'
    : scanning
      ? 'History is still being indexed; this list may be incomplete.'
      : degraded
        ? 'Live indexing is interrupted; this list may be out of date.'
        : incomplete
          ? 'Some history could not be fully indexed.'
          : null;

  return (
    <section className="xt-sessions">
      <div className="xt-sessions-heading">
        <div>
          <h1>Sessions</h1>
          <p>Indexed history on this Mac</p>
        </div>
        <div className="xt-sessions-filters">
          <Search
            label="Search sessions"
            placeholder="Search ID, repository or branch…"
            value={input}
            onValueChange={setInput}
            maxLength={256}
          />
          <select
            aria-label="Filter by host"
            value={host ?? ''}
            onChange={(e) => setHost(e.target.value || null)}
          >
            <option value="">All hosts</option>
            <option value="claude">Claude</option>
            <option value="codex">Codex</option>
            <option value="cursor">Cursor</option>
          </select>
        </div>
      </div>
      {coverageNotice && (
        <p className="xt-sessions-notice" role="status">
          {coverageNotice} <Link to="/settings">View indexing status</Link>
        </p>
      )}
      {params.has('pr') ? (
        <p role="status">
          Pull request filtering is not available yet. <Link to="/sessions">View all sessions</Link>
        </p>
      ) : source.kind === 'preview' ? (
        <p>Open the desktop app to browse your indexed sessions.</p>
      ) : (
        <SectionCard
          title="Session history"
          meta={`${rows.length} loaded`}
          right={
            <Button
              variant="outline"
              height={28}
              disabled={query.isFetching}
              onClick={() => void query.refetch()}
            >
              Refresh
            </Button>
          }
        >
          {query.isError ? (
            <p role="alert">
              Sessions could not be loaded.{' '}
              <button onClick={() => void query.refetch()}>Try again</button>
            </p>
          ) : (
            <DataTable
              label="Indexed sessions"
              columns={columns}
              rows={rows}
              getRowKey={(row) => row.id}
              rowHeight={44}
              loading={query.isPending}
              emptyMessage={
                search || host
                  ? 'No sessions match these filters.'
                  : 'No indexed sessions yet. Check indexing status in Settings.'
              }
              minWidth={850}
              maxHeight="60vh"
            />
          )}
          {query.hasNextPage && (
            <div className="xt-sessions-more">
              <Button disabled={query.isFetching} onClick={() => void query.fetchNextPage()}>
                Load more sessions
              </Button>
            </div>
          )}
        </SectionCard>
      )}
    </section>
  );
}
