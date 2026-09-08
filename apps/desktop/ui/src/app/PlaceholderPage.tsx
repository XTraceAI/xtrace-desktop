import { useLocation, useParams, useSearchParams } from 'react-router';
import { useData } from '../data/DataProvider';
import { MetricCell } from '../kit/MetricCell';
import { SectionCard } from '../kit/SectionCard';
import { useAppInfo, useDbCounts } from './useAppInfo';

export function PlaceholderPage({ title }: { title: string }) {
  const { pathname } = useLocation();
  const { ruleId } = useParams();
  const [params] = useSearchParams();
  const info = useAppInfo();
  const counts = useDbCounts();
  const { source } = useData();
  return (
    <section className="xt-placeholder">
      <h1>{title}</h1>
      <p className="xt-page-subline">This view is coming next.</p>
      {ruleId && <p className="xt-page-context">Rule: {ruleId}</p>}
      {pathname === '/sessions' && params.has('pr') && (
        <p className="xt-page-context">Pull request filter: {params.get('pr')}</p>
      )}
      {pathname === '/settings' && source.kind !== 'preview' && (
        <SectionCard title="Local database">
          <dl className="xt-database-summary">
            <dt>Data folder</dt>
            <dd>{info.data?.data_dir ?? 'Unavailable'}</dd>
            <dt>Schema version</dt>
            <dd>
              <MetricCell value={info.data?.schema_version} />
            </dd>
            <dt>Sessions</dt>
            <dd>
              <MetricCell value={counts.data?.sessions} reason="Database counts are unavailable" />
            </dd>
            <dt>Records</dt>
            <dd>
              <MetricCell value={counts.data?.records} reason="Database counts are unavailable" />
            </dd>
            <dt>Usage rows</dt>
            <dd>
              <MetricCell value={counts.data?.usage} reason="Database counts are unavailable" />
            </dd>
          </dl>
          {counts.isError && <p role="alert">Database counts could not be loaded.</p>}
        </SectionCard>
      )}
    </section>
  );
}
