import { Fragment } from 'react';
import { useLocation, useParams, useSearchParams } from 'react-router';
import { useData } from '../data/DataProvider';
import { useTheme, type ThemePreference } from '../theme/ThemeProvider';
import { Button } from '../kit/Button';
import { MetricCell } from '../kit/MetricCell';
import { SectionCard } from '../kit/SectionCard';
import { useAppInfo, useDbCounts, useNativeIndexStatus } from './useAppInfo';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';

export function PlaceholderPage({ title }: { title: string }) {
  const { pathname } = useLocation();
  const { ruleId } = useParams();
  const [params] = useSearchParams();
  const info = useAppInfo();
  const counts = useDbCounts();
  const index = useNativeIndexStatus();
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
      {pathname === '/settings' && source.kind !== 'preview' && (
        <SectionCard title="Native index">
          {index.data ? (
            <NativeIndexSummary status={index.data} />
          ) : index.isError ? (
            <p role="alert">Native index status could not be loaded.</p>
          ) : (
            <p role="status">Reading native index status…</p>
          )}
        </SectionCard>
      )}
    </section>
  );
}

const phaseText = (status: NativeIndexStatus) => {
  const { phase } = status;
  switch (phase.phase) {
    case 'disabled':
      return `Disabled: ${phase.reason}`;
    case 'scanning':
      return `Scanning (${status.files_scanned} Claude transcripts read)`;
    case 'ready':
      return `Ready (${status.reconciles} reconciliations)`;
    case 'stopped':
      return 'Stopped';
  }
};
const freshnessText = (status: NativeIndexStatus) => {
  const { freshness } = status;
  switch (freshness.freshness) {
    case 'unknown':
      return 'Not watching yet';
    case 'live':
      return 'Live: changes are reconciled as they happen';
    case 'degraded':
      return `Degraded: ${freshness.reason}`;
  }
};
const hostText = (host: NativeIndexStatus['hosts'][number]) => {
  const counts = `${host.sessions_imported} imported, ${host.sessions_partial} partial, ${host.sessions_skipped} skipped, ${host.records_new} new records`;
  return host.detail ? `${host.state} · ${counts} · ${host.detail}` : `${host.state} · ${counts}`;
};

/** The typed status the app publishes; every field is shown as reported, never invented. */
function NativeIndexSummary({ status }: { status: NativeIndexStatus }) {
  return (
    <dl className="xt-native-index-summary" data-testid="native-index">
      <dt>State</dt>
      <dd>{phaseText(status)}</dd>
      <dt>Freshness</dt>
      <dd>{freshnessText(status)}</dd>
      <dt>Python</dt>
      <dd>
        {status.python.state === 'available'
          ? status.python.path
          : status.python.state === 'resolving'
            ? 'Resolving…'
            : `Unavailable: ${status.python.reason}`}
      </dd>
      <dt>Readers</dt>
      <dd>
        {status.readers.state === 'verified'
          ? `Bundled memhub ${status.readers.plugin_version} at ${status.readers.commit.slice(0, 12)}`
          : `Unavailable: ${status.readers.reason}`}
      </dd>
      {status.hosts.map((host) => (
        <Fragment key={host.host}>
          <dt>{host.host}</dt>
          <dd>{hostText(host)}</dd>
        </Fragment>
      ))}
    </dl>
  );
}
