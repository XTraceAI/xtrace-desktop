import { useId, useRef } from 'react';
import type { TelemetryControls } from '../../data/DataSource';
import type { TelemetryState } from '../../data/generated/TelemetryState';
import { Button } from '../../kit/Button';
import { Icon } from '../../kit/icons';
import { EventListLink, OffUntilRestart, useAcknowledgeWhenVisible } from './TelemetryDisclosure';
import { useTelemetry } from './telemetry-query';
import '../../styles/telemetry.css';

/** The pop-up's checklist. Every item matches what the app sends (TELEMETRY.md). */
export const SENT_ITEMS = [
  'App and macOS version',
  'Coding agents used (Claude, Codex, Cursor)',
  'A random ID made on this Mac',
] as const;
export const NEVER_SENT_ITEMS = [
  'Code or file paths',
  'Prompts or conversation text',
  'Repo names, PR links, token counts or costs',
] as const;

/**
 * The disclosure as the main window's floating pop-up: one flat surface with
 * a title, what is and is never sent, a "Got it" that only closes it, a quiet
 * Turn off and the full list. The same rules as the page disclosure: it never asks for or implies
 * consent, closing changes nothing, and it counts as shown only once visible.
 */
export function TelemetryPopup({
  controls,
  state,
  onClose,
}: {
  controls: TelemetryControls;
  state: TelemetryState;
  onClose?: () => void;
}) {
  const ref = useRef<HTMLElement>(null);
  const titleId = useId();
  const { setEnabled, acknowledge } = useTelemetry(controls);
  useAcknowledgeWhenVisible(ref, state.show_disclosure, () => acknowledge.mutate());
  // Closes the pop-up and nothing else: it saves no choice and records no
  // consent, exactly like Escape.
  const gotIt = onClose && (
    <Button variant="primary" height={28} onClick={onClose}>
      Got it
    </Button>
  );
  return (
    <section
      ref={ref}
      className="xt-telemetry-popup"
      aria-label="Anonymous usage data"
      data-testid="telemetry-disclosure"
    >
      <header className="xt-telemetry-popup-head">
        <h2 id={titleId}>
          {state.preference_on
            ? 'Anonymous usage data is on by default'
            : 'Anonymous usage data is off'}
        </h2>
        <p className="xt-telemetry-popup-sub">
          Counts app opens and how often it's used, to improve XTrace.
        </p>
      </header>
      <div className="xt-telemetry-checklist">
        <Checklist title="Sent" items={SENT_ITEMS} icon="check" />
        <Checklist title="Never sent" items={NEVER_SENT_ITEMS} icon="close" />
      </div>
      {state.status === 'off_until_restart' ? (
        <>
          <OffUntilRestart
            pending={setEnabled.isPending}
            onRetry={() => setEnabled.mutate(false)}
          />
          {gotIt}
        </>
      ) : state.preference_on ? (
        <div className="xt-telemetry-popup-actions">
          {gotIt}
          <Button
            variant="ghost"
            height={28}
            disabled={setEnabled.isPending}
            onClick={() => setEnabled.mutate(false)}
          >
            Turn off
          </Button>
          <EventListLink controls={controls} inline>
            Full list
          </EventListLink>
        </div>
      ) : (
        <div className="xt-telemetry-popup-actions">
          <p role="status" className="xt-telemetry-note" data-testid="telemetry-off">
            Telemetry is off. Nothing is sent.
          </p>
          {gotIt}
        </div>
      )}
      {setEnabled.isError && <p role="alert">Telemetry could not be turned off.</p>}
    </section>
  );
}

function Checklist({
  title,
  items,
  icon,
}: {
  title: string;
  items: readonly string[];
  icon: 'check' | 'close';
}) {
  return (
    <div className="xt-telemetry-list" data-kind={icon === 'check' ? 'sent' : 'never'}>
      <h3>{title}</h3>
      <ul>
        {items.map((item) => (
          <li key={item}>
            <Icon name={icon} size={12} className="xt-telemetry-mark" />
            {item}
          </li>
        ))}
      </ul>
    </div>
  );
}
