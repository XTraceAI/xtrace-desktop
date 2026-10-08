import {
  LIVE_SESSION_HOSTS,
  type LiveSessionHost,
  type LiveSessionStatus,
} from './live-session-status';
import { HostGlyph } from '../kit/HostGlyph';
import { isLiveSessionHost } from './live-session-status';
import '../styles/live-session-status.css';

const labels: Record<LiveSessionStatus, string> = {
  running: 'Running',
  waiting_approval: 'Waiting for approval',
  waiting_input: 'Waiting for input',
  idle: 'Idle',
  unknown: 'Unknown',
};

export function LiveSessionBadge({
  status,
  host = 'codex',
}: {
  status: LiveSessionStatus | undefined;
  host?: LiveSessionHost;
}) {
  if (status === undefined || status === 'idle' || status === 'unknown') return null;
  return (
    <span
      className="xt-live-session-badge"
      data-live-status={status}
      role="status"
      aria-live="off"
      aria-label={`${LIVE_SESSION_HOSTS[host].label} · ${labels[status]}`}
      title={`${LIVE_SESSION_HOSTS[host].source} · ${labels[status]}`}
    >
      {labels[status]}
    </span>
  );
}

/** The same active-session arc wherever a table shows the host. */
export function LiveSessionHost({ host, status }: { host: string; status?: LiveSessionStatus }) {
  const glyph = <HostGlyph host={host} size={18} />;
  return status === 'running' && isLiveSessionHost(host) ? (
    <span
      className="xt-lane-live-host"
      data-live-status="running"
      role="img"
      aria-label={`${LIVE_SESSION_HOSTS[host].label} · Running`}
      title={`${LIVE_SESSION_HOSTS[host].source} · Running`}
    >
      <span aria-hidden="true">{glyph}</span>
    </span>
  ) : (
    glyph
  );
}
