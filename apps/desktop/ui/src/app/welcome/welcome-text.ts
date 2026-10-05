import type { NativeHostStatus } from '../../data/generated/NativeHostStatus';
import type { NativeHostState } from '../../data/generated/NativeHostState';
import type { NativeIndexStatus } from '../../data/generated/NativeIndexStatus';
import type { ControlTone } from '../../kit/control-tone';

const names: Record<string, string> = { claude: 'Claude', codex: 'Codex', cursor: 'Cursor' };
export const hostName = (host: string) => names[host] ?? host;

/**
 * What the last scan of one host's local history did. These are outcomes of
 * reading files on this Mac: none of them says whether an agent is installed,
 * connected or capturing now.
 */
export const hostOutcome: Record<NativeHostState, { label: string; tone: ControlTone }> = {
  pending: { label: 'Not read yet', tone: 'meta' },
  complete: { label: 'Read', tone: 'success' },
  incomplete: { label: 'Read with gaps', tone: 'warning' },
  missing_source: { label: 'No local history found', tone: 'meta' },
  missing_runtime: { label: 'Needs Python 3', tone: 'warning' },
  pin_mismatch: { label: 'Reader not verified', tone: 'danger' },
  reader_failed: { label: 'Reader failed', tone: 'danger' },
  cancelled: { label: 'Stopped before finishing', tone: 'meta' },
};

const needsAttention: ReadonlySet<NativeHostState> = new Set([
  'incomplete',
  'missing_runtime',
  'pin_mismatch',
  'reader_failed',
]);
/** Hosts whose last scan left history unread or partly read. */
export const attentionCount = (hosts: readonly NativeHostStatus[]) =>
  hosts.filter((host) => needsAttention.has(host.state)).length;

/** How a source's history is read; Claude transcripts need no interpreter. */
export const readerNote = (host: string) =>
  host === 'claude'
    ? 'Transcripts are read directly; no Python needed.'
    : 'Read with the bundled reader and Python 3.';

export function phasePill(status: NativeIndexStatus): { label: string; tone: ControlTone } {
  switch (status.phase.phase) {
    case 'scanning':
      return { label: 'Scanning', tone: 'accent' };
    case 'ready':
      return attentionCount(status.hosts) > 0
        ? { label: 'Ready with problems', tone: 'warning' }
        : { label: 'Ready', tone: 'success' };
    case 'disabled':
      return { label: 'Not running', tone: 'meta' };
    case 'stopped':
      return { label: 'Stopped', tone: 'meta' };
  }
}

export const plural = (value: number, one: string, many = `${one}s`) =>
  `${value.toLocaleString('en-US')} ${value === 1 ? one : many}`;
