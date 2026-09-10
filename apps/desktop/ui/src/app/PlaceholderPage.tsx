import { useLocation, useParams, useSearchParams } from 'react-router';
import { useData } from '../data/DataProvider';
import { useTheme, type ThemePreference } from '../theme/ThemeProvider';
import { Button } from '../kit/Button';
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
  const { preference, setPreference } = useTheme();
  return (
    <section className="xt-placeholder">
      <h1>{title}</h1>
      <p className="xt-page-subline">This view is coming next.</p>
      {ruleId && <p className="xt-page-context">Rule: {ruleId}</p>}
      {pathname === '/sessions' && params.has('pr') && (
        <p className="xt-page-context">Pull request filter: {params.get('pr')}</p>
      )}
      {pathname === '/settings' && (
        <SectionCard title="Appearance">
          <label className="xt-appearance">
            Theme
            <select
              aria-label="Appearance"
              value={preference}
              onChange={(event) => setPreference(event.target.value as ThemePreference)}
            >
              <option value="system">System</option>
              <option value="dark">Dark</option>
              <option value="light">Light</option>
            </select>
          </label>
        </SectionCard>
      )}
      {pathname === '/settings' && source.kind !== 'preview' && (
        <SectionCard
          title="Local database"
          right={
            <Button
              variant="outline"
              height={28}
              disabled={info.isFetching || counts.isFetching}
              onClick={() => {
                void info.refetch();
                void counts.refetch();
              }}
            >
              Refresh
            </Button>
          }
        >
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
