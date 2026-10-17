import { useEffect, useRef, useState, type RefObject } from 'react';
import type { TelemetryControls } from '../../data/DataSource';
import type { TelemetryState } from '../../data/generated/TelemetryState';
import { Button } from '../../kit/Button';
import { SectionCard } from '../../kit/SectionCard';
import { useTelemetry } from './telemetry-query';
import '../../styles/telemetry.css';

/**
 * The published list of every event and property: only the browser preview's
 * href. The app opens its own version's page through the native command.
 */
export const TELEMETRY_DOC_URL =
  'https://github.com/XTraceAI/xtrace-desktop/blob/main/TELEMETRY.md';

/** At least this share of the disclosure must be on screen to count as shown. */
const VISIBLE_SHARE = 0.5;

/**
 * Calls `acknowledge` once, after mount, and only while the element is on
 * screen in a visible window: rendered but hidden, scrolled away or in a
 * hidden window does not count. Without an IntersectionObserver only the
 * window's visibility is checked.
 */
export function useAcknowledgeWhenVisible(
  ref: RefObject<HTMLElement | null>,
  enabled: boolean,
  acknowledge: () => void,
) {
  const latest = useRef(acknowledge);
  useEffect(() => {
    latest.current = acknowledge;
  });
  useEffect(() => {
    const element = ref.current;
    if (!enabled || !element) return;
    let done = false;
    let onScreen = typeof IntersectionObserver !== 'function';
    const check = () => {
      if (done || !onScreen || document.visibilityState !== 'visible' || !element.isConnected)
        return;
      done = true;
      latest.current();
    };
    const observer = onScreen
      ? null
      : new IntersectionObserver(
          (entries) => {
            onScreen = entries.some((entry) => entry.intersectionRatio >= VISIBLE_SHARE);
            check();
          },
          { threshold: [0, VISIBLE_SHARE, 1] },
        );
    observer?.observe(element);
    document.addEventListener('visibilitychange', check);
    check();
    return () => {
      done = true;
      observer?.disconnect();
      document.removeEventListener('visibilitychange', check);
    };
  }, [ref, enabled]);
}

/**
 * The telemetry disclosure: on by default, what is and is never sent, the
 * full list, and a one-click Turn off. It never asks for or implies consent;
 * leaving it open or closing it changes nothing. The app records that it was
 * shown only once it is visible, and only then can anything be sent.
 */
export function TelemetryDisclosure({
  controls,
  state,
}: {
  controls: TelemetryControls;
  state: TelemetryState;
}) {
  const ref = useRef<HTMLDivElement>(null);
  const { setEnabled, acknowledge } = useTelemetry(controls);
  useAcknowledgeWhenVisible(ref, state.show_disclosure, () => acknowledge.mutate());
  return (
    <SectionCard title="Anonymous usage data">
      <div ref={ref} className="xt-telemetry" data-testid="telemetry-disclosure">
        <p className="xt-telemetry-lead">
          Telemetry is on by default. XTrace sends a few anonymous events, such as when the app is
          opened and how often it is used, so we can count how many people use it.
        </p>
        <TelemetryFacts />
        <EventListLink controls={controls} />
        {state.status === 'off_until_restart' ? (
          <OffUntilRestart
            pending={setEnabled.isPending}
            onRetry={() => setEnabled.mutate(false)}
          />
        ) : state.preference_on ? (
          <div className="xt-telemetry-actions">
            <Button
              variant="outline"
              height={28}
              disabled={setEnabled.isPending}
              onClick={() => setEnabled.mutate(false)}
            >
              Turn off
            </Button>
            <span className="xt-telemetry-note">You can also change this later in Settings.</span>
          </div>
        ) : (
          <p role="status" className="xt-telemetry-note" data-testid="telemetry-off">
            Telemetry is off. Nothing is sent. You can turn it back on in Settings.
          </p>
        )}
        {setEnabled.isError && <p role="alert">Telemetry could not be turned off.</p>}
      </div>
    </SectionCard>
  );
}

/**
 * The app reports telemetry off for this launch only: turning it off could
 * not be saved, so the saved choice, still on, applies after a restart.
 */
export function OffUntilRestart({ pending, onRetry }: { pending: boolean; onRetry: () => void }) {
  return (
    <div className="xt-telemetry-actions" data-testid="telemetry-off-until-restart">
      <p role="status" className="xt-telemetry-note">
        Telemetry is off for now, but turning it off could not be saved. It will be on again when
        the app restarts.
      </p>
      <Button variant="outline" height={28} disabled={pending} onClick={onRetry}>
        Try turning off again
      </Button>
    </div>
  );
}

/** What is sent and what never is; the same words on every screen. */
export function TelemetryFacts() {
  return (
    <dl className="xt-telemetry-facts">
      <dt>Sent with each event</dt>
      <dd>
        The app version, the macOS version, which coding agents have history on this Mac (Claude,
        Codex, Cursor), and a random ID created on this Mac.
      </dd>
      <dt>Never sent</dt>
      <dd>
        Repository names, file paths, prompts or other conversation text, pull-request links, token
        counts or costs.
      </dd>
    </dl>
  );
}

/**
 * Opens the published list outside the app; a plain new-tab link elsewhere.
 * `inline` leaves out the paragraph around it, for a row of actions.
 */
export function EventListLink({
  controls,
  inline = false,
  children = 'See every event and property (TELEMETRY.md)',
}: {
  controls?: TelemetryControls;
  inline?: boolean;
  children?: string;
}) {
  const [failed, setFailed] = useState(false);
  const link = (
    <a
      className="xt-settings-link"
      href={TELEMETRY_DOC_URL}
      target="_blank"
      rel="noopener noreferrer"
      onClick={(event) => {
        if (!controls) return;
        // The app window never navigates: the system browser opens the page.
        event.preventDefault();
        setFailed(false);
        controls.openEventList().catch(() => setFailed(true));
      }}
    >
      {children}
    </a>
  );
  return (
    <>
      {inline ? link : <p className="xt-telemetry-note">{link}</p>}
      {failed && <p role="alert">The event list could not be opened in your browser.</p>}
    </>
  );
}
