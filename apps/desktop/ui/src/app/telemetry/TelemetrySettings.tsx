import type { TelemetryControls } from '../../data/DataSource';
import type { TelemetryStatus } from '../../data/generated/TelemetryStatus';
import { SectionCard } from '../../kit/SectionCard';
import { Toggle } from '../../kit/Toggle';
import { EventListLink, OffUntilRestart, TelemetryFacts } from './TelemetryDisclosure';
import { useTelemetry } from './telemetry-query';
import '../../styles/telemetry.css';

/** The words for each state the app reports. */
export const telemetryStatusText: Record<TelemetryStatus, string> = {
  environment_off: 'Off for this launch: XTRACE_TELEMETRY turns it off',
  off_until_restart: 'Off until the app restarts: turning it off could not be saved',
  not_configured: 'Nothing is sent: this copy of the app does not send telemetry',
  turned_off: 'Off',
  awaiting_disclosure: 'On, but nothing is sent until the notice about it has been shown',
  sending: 'On',
};

/** The Settings switch over the same saved preference the disclosure changes. */
export function TelemetrySettings({ controls }: { controls?: TelemetryControls }) {
  return (
    <SectionCard title="Telemetry">
      {controls ? (
        <TelemetryControl controls={controls} />
      ) : (
        <p className="xt-settings-note">
          Telemetry is saved in the desktop app&apos;s database. This preview sends nothing.
        </p>
      )}
      <TelemetryFacts />
      <EventListLink controls={controls} />
    </SectionCard>
  );
}

function TelemetryControl({ controls }: { controls: TelemetryControls }) {
  const { state, setEnabled } = useTelemetry(controls);
  const saved = state.data;
  return (
    <>
      <dl className="xt-database-summary">
        <dt>Status</dt>
        <dd data-testid="telemetry-status">
          {saved ? telemetryStatusText[saved.status] : state.isError ? 'Unavailable' : 'Reading…'}
        </dd>
      </dl>
      <div className="xt-settings-control">
        <Toggle
          label="Send anonymous usage data"
          checked={saved?.enabled ?? false}
          disabled={!saved || !saved.can_change || setEnabled.isPending}
          onChange={(checked) => setEnabled.mutate(checked)}
        />
      </div>
      {saved?.status === 'off_until_restart' && (
        <div className="xt-settings-control">
          <OffUntilRestart
            pending={setEnabled.isPending}
            onRetry={() => setEnabled.mutate(false)}
          />
        </div>
      )}
      {saved?.env_off && (
        <p className="xt-settings-note" data-testid="telemetry-env">
          The XTRACE_TELEMETRY environment variable turns telemetry off for this launch. The switch
          cannot change that; start the app without it to use your saved choice.
        </p>
      )}
      <p className="xt-settings-note">
        On by default. Turning it off takes effect at once: events waiting to be sent are dropped
        and are not sent later.
      </p>
      {state.isError && <p role="alert">The telemetry setting could not be read.</p>}
      {setEnabled.isError && <p role="alert">The telemetry setting could not be changed.</p>}
    </>
  );
}
