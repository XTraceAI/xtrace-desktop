import {
  LIVE_SESSION_HOSTS,
  type LiveSessionHost,
  type LiveSessionStatus,
} from './live-session-status';
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
  if (status === undefined) return null;
  return (
    <span
      className="xt-live-session-badge"
      data-live-status={status}
      role="status"
      aria-live="off"
      aria-label={`${LIVE_SESSION_HOSTS[host].label} · ${labels[status]}`}
      title={
        status === 'unknown'
          ? `${LIVE_SESSION_HOSTS[host].source} · live state unavailable`
          : `${LIVE_SESSION_HOSTS[host].source} · ${labels[status]}`
      }
    >
      {labels[status]}
    </span>
  );
}
