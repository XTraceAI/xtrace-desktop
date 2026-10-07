import { useNavigate } from 'react-router';
import { useData } from '../../data/DataProvider';
import type { DbCounts } from '../../data/generated/DbCounts';
import type { NativeHostStatus } from '../../data/generated/NativeHostStatus';
import type { NativeIndexStatus } from '../../data/generated/NativeIndexStatus';
import { StatePill } from '../../kit/Badge';
import { Button } from '../../kit/Button';
import { count } from '../../kit/format';
import { HostGlyph } from '../../kit/HostGlyph';
import { MetricCell } from '../../kit/MetricCell';
import { SectionCard } from '../../kit/SectionCard';
import { hostScanText } from '../index-host-state';
import { freshnessText } from '../native-index-text';
import { useDbCounts, useNativeIndexStatus } from '../useAppInfo';
import { completeWelcome } from './welcome-completion';
import { attentionCount, hostName, phasePill, plural, readerNote } from './welcome-text';
import '../../styles/welcome.css';

const TITLE = 'Welcome to XTrace';

/**
 * First launch: what the local index is doing and what each local history
 * source's last scan found, from the status the app already publishes. Every
 * value is read, not estimated; the Dashboard is always one click away.
 */
export function WelcomePage() {
  const { source } = useData();
  const navigate = useNavigate();
  const index = useNativeIndexStatus();
  const counts = useDbCounts();
  const preview = source.kind === 'preview';
  const failed = index.isError || counts.isError;
  const retry = () => {
    if (index.isError) void index.refetch();
    if (counts.isError) void counts.refetch();
  };
  // The explicit continue: stored when storage allows, and never a gate.
  const open = () => {
    completeWelcome();
    void navigate('/dashboard');
  };
  return (
    <section className="xt-welcome" aria-busy={index.isFetching || counts.isFetching || undefined}>
      <header className="xt-welcome-head">
        <h1>{TITLE}</h1>
        <p className="xt-welcome-subline">
          XTrace reads the agent history already on this Mac. No plugin, account or cloud service is
          needed to see it.
        </p>
      </header>
      <div className="xt-welcome-grid">
        <SectionCard
          title="Local index"
          right={index.data && <PhasePill status={index.data} />}
          padding="14px"
        >
          <div className="xt-welcome-index">
            {preview ? (
              <p className="xt-welcome-status">Open the desktop app to read local history.</p>
            ) : index.data ? (
              <IndexFacts status={index.data} />
            ) : index.isError ? null : (
              <p role="status" className="xt-welcome-status">
                Reading local index status…
              </p>
            )}
            {!preview && <CountTiles counts={counts.data} failed={counts.isError} />}
            {failed && (
              <p role="alert" className="xt-welcome-alert">
                {index.isError && counts.isError
                  ? 'Local index status and database counts could not be read.'
                  : index.isError
                    ? 'Local index status could not be read.'
                    : 'Database counts could not be read.'}{' '}
                <Button
                  variant="outline"
                  height={28}
                  disabled={index.isFetching || counts.isFetching}
                  onClick={retry}
                >
                  Retry
                </Button>
              </p>
            )}
            <div className="xt-welcome-spacer" />
            <Button variant="primary" height={34} className="xt-welcome-cta" onClick={open}>
              Open Dashboard
            </Button>
          </div>
        </SectionCard>
        <SectionCard
          title="Local history sources"
          meta={index.data && sourcesMeta(index.data)}
          padding="12px"
        >
          {preview ? (
            <p className="xt-welcome-status">The desktop app lists what it finds on this Mac.</p>
          ) : index.data ? (
            <Sources status={index.data} />
          ) : index.isError ? (
            <p className="xt-welcome-status">Sources are unknown until the status is read.</p>
          ) : (
            <p className="xt-welcome-status">Reading sources…</p>
          )}
        </SectionCard>
      </div>
    </section>
  );
}

function PhasePill({ status }: { status: NativeIndexStatus }) {
  const pill = phasePill(status);
  return (
    <StatePill tone={pill.tone} live={status.phase.phase === 'scanning'}>
      {pill.label}
    </StatePill>
  );
}

const sourcesMeta = (status: NativeIndexStatus) => {
  if (status.hosts.length === 0) return undefined;
  const read = status.hosts.filter((h) => h.state === 'complete' || h.state === 'incomplete');
  return `${read.length} of ${status.hosts.length} read · last scan since launch`;
};

function IndexFacts({ status }: { status: NativeIndexStatus }) {
  const { phase } = status;
  const attention = attentionCount(status.hosts);
  return (
    <div className="xt-welcome-facts">
      <p className="xt-welcome-status" data-testid="welcome-phase">
        {phase.phase === 'scanning'
          ? 'The initial scan of your local history is running. Counts below grow as it goes.'
          : phase.phase === 'ready'
            ? attention > 0
              ? `The initial scan finished; ${plural(attention, 'source')} reported a problem.`
              : 'The initial scan finished.'
            : phase.phase === 'disabled'
              ? `The local index is not running: ${phase.reason}`
              : 'The local index has stopped.'}
      </p>
      <dl className="xt-welcome-dl">
        <dt>Claude transcripts read</dt>
        <dd data-testid="welcome-files">{count(status.files_scanned)}</dd>
        <dt>Updates</dt>
        <dd
          data-tone={status.freshness.freshness === 'degraded' ? 'warning' : undefined}
          data-testid="welcome-freshness"
        >
          {freshnessText(status)}
        </dd>
      </dl>
      <p className="xt-welcome-hint">
        Transcript files are counted as they are read; the total is not known in advance.
      </p>
    </div>
  );
}

function CountTiles({ counts, failed }: { counts: DbCounts | undefined; failed: boolean }) {
  const reason = failed ? 'Database counts could not be read' : 'Reading database counts';
  return (
    <div className="xt-welcome-counts" role="group" aria-label="In the local database now">
      {(
        [
          ['Sessions', counts?.sessions],
          ['Records', counts?.records],
          ['Usage rows', counts?.usage],
        ] as const
      ).map(([label, value]) => (
        <div key={label} className="xt-welcome-count">
          <span className="xt-welcome-count-label">{label}</span>
          <MetricCell value={value} format={count} size={18} reason={reason} />
        </div>
      ))}
      <span className="xt-welcome-count-caption">In the local database now</span>
    </div>
  );
}

function Sources({ status }: { status: NativeIndexStatus }) {
  if (status.hosts.length === 0)
    return (
      <p className="xt-welcome-status">
        {status.phase.phase === 'disabled'
          ? 'No sources are read while the local index is not running.'
          : 'No sources have been reported yet.'}
      </p>
    );
  const { python, readers } = status;
  return (
    <div className="xt-welcome-sources">
      <ul className="xt-welcome-hosts" aria-label="Local history sources">
        {status.hosts.map((host) => (
          <HostCard key={host.host} host={host} />
        ))}
      </ul>
      <dl className="xt-welcome-dl xt-welcome-runtime">
        <dt>Python</dt>
        <dd data-tone={python.state === 'missing' ? 'warning' : undefined}>
          {python.state === 'available'
            ? `Found at ${python.path}`
            : python.state === 'resolving'
              ? 'Looking…'
              : `Not found: ${python.reason}. Codex and Cursor history needs it; Claude history is read without it.`}
        </dd>
        <dt>Readers</dt>
        <dd data-tone={readers.state === 'unavailable' ? 'warning' : undefined}>
          {readers.state === 'verified'
            ? 'Bundled readers verified'
            : `Bundled readers unavailable: ${readers.reason}`}
        </dd>
      </dl>
    </div>
  );
}

function HostCard({ host }: { host: NativeHostStatus }) {
  const name = hostName(host.host);
  const outcome = hostScanText(host);
  const read = host.state === 'complete' || host.state === 'incomplete';
  return (
    <li className="xt-welcome-host" aria-label={`${name}: ${outcome.label}`}>
      <div className="xt-welcome-host-head">
        <HostGlyph host={host.host} size={30} />
        <span className="xt-welcome-host-name">{name}</span>
      </div>
      <StatePill tone={outcome.tone}>{outcome.label}</StatePill>
      {read && (
        <dl className="xt-welcome-dl xt-welcome-host-facts">
          <dt>Sessions</dt>
          <dd>
            {count(host.sessions_imported)}
            {host.sessions_partial > 0 && ` · ${count(host.sessions_partial)} partial`}
            {host.sessions_skipped > 0 && ` · ${count(host.sessions_skipped)} skipped`}
          </dd>
          <dt>New records</dt>
          <dd>{count(host.records_new)}</dd>
          {host.diagnostics > 0 && (
            <>
              <dt>Diagnostics</dt>
              <dd data-tone="warning">{count(host.diagnostics)}</dd>
            </>
          )}
        </dl>
      )}
      {host.detail && (
        <p className="xt-welcome-host-detail" title={host.detail}>
          {host.detail}
        </p>
      )}
      <p className="xt-welcome-hint">{readerNote(host.host)}</p>
    </li>
  );
}
