import type { NativeHostState } from '../data/generated/NativeHostState';
import type { NativeHostStatus } from '../data/generated/NativeHostStatus';
import type { ControlTone } from '../kit/control-tone';

/**
 * One host's last scan of local history, in words: the one label map the
 * sidebar, Welcome and Settings share. Whether a scan is a problem is not
 * decided here: the app sends it as `needs_attention` (decided once, in
 * `native_index::mark_attention`), and every screen reads that flag.
 *
 * These are outcomes of reading files on this Mac: none of them says whether
 * an agent is installed, connected or capturing now.
 */
export type HostScanText = { label: string; tone: ControlTone };

/**
 * Per state: the label, and which problem colour or which fine colour it takes.
 * Which of the two applies is never read from this table: it is the app's
 * `needs_attention`, so a host the app does not flag never gets a problem
 * colour, and one it flags never gets a fine colour.
 */
type StateText = {
  label: string;
  problem?: 'warning' | 'danger';
  fine?: 'success' | 'accent' | 'meta';
};

const states: Record<NativeHostState, (needsAttention: boolean) => StateText> = {
  // Not scanned yet: waiting while the scan runs, unread once none does.
  pending: (needsAttention) =>
    needsAttention ? { label: 'Not read yet' } : { label: 'Waiting to be read', fine: 'accent' },
  complete: () => ({ label: 'Read', fine: 'success' }),
  incomplete: () => ({ label: 'Read with gaps' }),
  // No source is not a failure: the host may simply have no local history.
  missing_source: () => ({ label: 'No local history found' }),
  missing_runtime: () => ({ label: 'Needs Python 3' }),
  pin_mismatch: () => ({ label: 'Reader not verified', problem: 'danger' }),
  reader_failed: () => ({ label: 'Reader failed', problem: 'danger' }),
  cancelled: () => ({ label: 'Stopped before finishing' }),
};

/** The label and colour of one host's last scan; whether it is a problem is the app's flag. */
export const hostScanText = (
  host: Pick<NativeHostStatus, 'state' | 'needs_attention'>,
): HostScanText => {
  const text = states[host.state](host.needs_attention);
  return {
    label: text.label,
    tone: host.needs_attention ? (text.problem ?? 'warning') : (text.fine ?? 'meta'),
  };
};
