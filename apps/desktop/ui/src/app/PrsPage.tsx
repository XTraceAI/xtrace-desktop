import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useSearchParams } from 'react-router';
import { useData } from '../data/DataProvider';
import { queryKeys } from '../data/query-client';
import { Segmented } from '../kit/Segmented';
import { rangeDays } from './dashboard/present';
import { useSelectedRange } from './dashboard/range';
import { PrAnalytics } from './PrAnalytics';
import { INVENTORY_VIEW, OVERLAP, PRS_VIEW } from './pr-analytics';
import { PrInventory } from './PrInventory';
import '../styles/prs.css';

type View = 'report' | typeof INVENTORY_VIEW;

/**
 * Two independent local reads under one heading. The report is the cached
 * per-PR linked-session analytics for the Shell's range and this page's
 * confidence mode; the inventory is every stored pull request a session still
 * links, over all time. Neither is computed from the other, and the range and
 * confidence mode apply to the report only. The view lives in the address, so
 * back and forward restore it; the report's confidence mode and filter
 * survive switching views.
 */
export function PrsPage() {
  const { source } = useData();
  const range = useSelectedRange();
  const days = rangeDays[range];
  const [params, setParams] = useSearchParams();
  const view: View = params.get(PRS_VIEW) === INVENTORY_VIEW ? INVENTORY_VIEW : 'report';
  const setView = (next: View) =>
    setParams((previous) => {
      const updated = new URLSearchParams(previous);
      if (next === INVENTORY_VIEW) updated.set(PRS_VIEW, INVENTORY_VIEW);
      else updated.delete(PRS_VIEW);
      return updated;
    });
  const [confirmedOnly, setConfirmedOnly] = useState(false);
  const [filter, setFilter] = useState('');
  // A local read of the cached report: committed imports, enrichment, pull
  // request refreshes and reconnects re-read it; nothing here refreshes a
  // pull request or reads a source.
  const report = useQuery({
    queryKey: queryKeys.prAnalytics(days, confirmedOnly),
    queryFn: () => source.pullRequestAnalytics(days, confirmedOnly),
    enabled: source.kind !== 'preview' && view === 'report',
  });

  return (
    <section className="xt-prs" data-view={view}>
      <div className="xt-prs-heading">
        <div>
          <h1>Pull requests</h1>
          <p>
            {view === 'report'
              ? `${OVERLAP} · rows are never summed`
              : 'Every indexed pull request a session links · cached facts, not live GitHub state'}
          </p>
        </div>
        <Segmented
          label="Pull requests view"
          options={[
            { value: 'report', label: 'Merged effort' },
            { value: INVENTORY_VIEW, label: 'Cached inventory' },
          ]}
          value={view}
          onChange={setView}
        />
      </div>
      {source.kind === 'preview' ? (
        <p>Open the desktop app to see its indexed pull requests.</p>
      ) : view === 'report' ? (
        <PrAnalytics
          query={report}
          confirmedOnly={confirmedOnly}
          onConfirmedOnly={setConfirmedOnly}
          filter={filter}
          onFilter={setFilter}
          onInventory={() => setView(INVENTORY_VIEW)}
        />
      ) : (
        <PrInventory />
      )}
    </section>
  );
}
