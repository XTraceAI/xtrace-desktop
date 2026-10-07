import type { NativeHostStatus } from '../data/generated/NativeHostStatus';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import type { SubscriptionState } from '../data/subscribe-invalidation';
import type { LocalIndexHost, LocalIndexStatus, SidebarProps } from '../kit/Sidebar';
import { hostName } from '../kit/hosts';
import { hostScanText } from './index-host-state';

/**
 * What the sidebar says about the local index: display words over the typed
 * status the app already publishes, and nothing the status does not hold. It
 * reads no plugin registry, receipt or surface, so it never says whether
 * anything is installed or capturing.
 */

/** The facts the sidebar shows. Counts, paths and reader pins stay in Settings. */
export type SidebarIndexFacts = Pick<NativeIndexStatus, 'phase' | 'freshness'> & {
  hosts: Pick<NativeHostStatus, 'host' | 'state' | 'needs_attention' | 'detail'>[];
};

/**
 * Narrows a status to those facts, so scan progress and reconciliation counts,
 * which the sidebar does not show, never re-render the Shell.
 */
export const sidebarIndexFacts = (status: NativeIndexStatus): SidebarIndexFacts => ({
  phase: status.phase,
  freshness: status.freshness,
  hosts: status.hosts.map(({ host, state, needs_attention, detail }) => ({
    host,
    state,
    needs_attention,
    detail,
  })),
});

/**
 * One host's row, and whether its last scan leaves the index short of that
 * host: the app's own `needs_attention`, never re-decided here.
 */
const hostScan = (
  host: SidebarIndexFacts['hosts'][number],
): { row: LocalIndexHost; open: boolean } => {
  const open = host.needs_attention;
  return {
    row: {
      host: hostName(host.host),
      state: hostScanText(host).label,
      ...(open && { attention: true }),
      ...(host.detail && { reason: host.detail }),
    },
    open,
  };
};

/** A label inside a sentence: its first letter lowered, the rest (“Python”) kept. */
const inSentence = (label: string) => label.charAt(0).toLowerCase() + label.slice(1);

type Described = Pick<LocalIndexStatus, 'label' | 'title' | 'tone' | 'summary'>;

/**
 * A reported reason closing a sentence: “: reason.” without doubling a full
 * stop it already has, and only the full stop when no reason was given.
 */
const because = (reason: string) => {
  const text = reason.trim();
  if (!text) return '.';
  return `: ${/[.!?]$/.test(text) ? text : `${text}.`}`;
};

/** What is said when the status cannot be read, or holds a state this build does not know. */
const unreadable: Described = {
  label: 'unknown',
  title: 'Status unavailable',
  tone: 'attention',
  summary: 'The local index status could not be read, so its state is unknown.',
};

/** The phase and watcher freshness as one state; host scans qualify it afterwards. */
function describe(status: SidebarIndexFacts, qualified: boolean): Described {
  const { phase, freshness } = status;
  switch (phase.phase) {
    case 'disabled':
      return {
        label: 'disabled',
        title: 'Disabled',
        tone: 'attention',
        summary: `Local history is not being indexed${because(phase.reason)}`,
      };
    case 'stopped':
      return {
        label: 'stopped',
        title: 'Stopped',
        tone: 'attention',
        summary: 'Local indexing has stopped. History indexed earlier is still shown.',
      };
    case 'scanning':
      return {
        label: 'scanning',
        title: 'Scanning',
        tone: 'idle',
        summary: 'The initial scan of local history is running. Numbers may be incomplete.',
      };
    case 'ready':
      break;
    default:
      // Never worded as ready: only the phases above are known to this build.
      return unreadable;
  }
  if (freshness.freshness === 'degraded')
    return {
      label: 'degraded',
      title: 'Updates interrupted',
      tone: 'attention',
      summary: `Watching for changes is degraded${because(freshness.reason)} The index may be out of date.`,
    };
  const live = freshness.freshness === 'live';
  const summary = live
    ? 'The local index is ready and changes are reconciled as they happen.'
    : 'The local index is ready. Watching for changes is not established, so new activity may not appear yet.';
  if (qualified)
    return {
      label: 'partial',
      title: live ? 'Updating · partial' : 'Ready · partial',
      tone: 'attention',
      summary,
    };
  return live
    ? { label: 'updating', title: 'Updating', tone: 'live', summary }
    : { label: 'ready', title: 'Ready', tone: 'idle', summary };
}

export type SidebarIndexInput = {
  /** The cached status; undefined until the first read answers. */
  status: SidebarIndexFacts | undefined;
  /** The latest status read failed. A status read earlier may still be cached. */
  readFailed: boolean;
  /** This renderer's live updates; only `connected` means status events are heard. */
  live: SubscriptionState;
  /**
   * The cached status was read since the listeners last registered. Hearing
   * events again is not that: until such a read succeeds the status is last known.
   */
  caughtUp: boolean;
};

export function sidebarIndexStatus({
  status,
  readFailed,
  live,
  caughtUp,
}: SidebarIndexInput): LocalIndexStatus {
  if (!status)
    return readFailed
      ? { ...unreadable, hosts: [] }
      : {
          label: 'checking',
          title: 'Checking',
          tone: 'idle',
          summary: 'Reading the local index status.',
          hosts: [],
        };
  const scans = status.hosts.map(hostScan);
  const open = scans.filter((scan) => scan.open).map((scan) => scan.row);
  const described = describe(status, open.length > 0);
  const notes: string[] = [];
  // A scan that did not complete says nothing about what earlier scans stored,
  // so this only says the index may fall short for those hosts.
  if (open.length > 0)
    notes.push(
      `Last scan not complete for ${open
        .map((row) => `${row.host} (${inSentence(row.state)})`)
        .join(', ')}. The index may be incomplete or out of date for ${
        open.length === 1 ? 'that host' : 'those hosts'
      }.`,
    );
  // A cached status is not proof that this renderer hears the index now: when
  // its events are not heard, or its last read failed, the state is last known.
  const unheard = live !== 'connected';
  // An unreadable state claims nothing, so there is nothing to mark as last known.
  const lastKnown = (unheard || readFailed || !caughtUp) && described !== unreadable;
  if (unheard)
    notes.push(
      live === 'failed'
        ? 'Live updates are unavailable, so this is the last status this window read and it may be out of date. Use Reconnect.'
        : 'Live updates are reconnecting, so this is the last status this window read and it may be out of date.',
    );
  else if (readFailed)
    notes.push(
      'The latest status read failed, so this is the last status this window read and it may be out of date.',
    );
  else if (!caughtUp)
    notes.push(
      'Live updates are back and the status is being read again, so this is the last status this window read and it may be out of date.',
    );
  return {
    label: lastKnown ? `${described.label}?` : described.label,
    title: lastKnown ? `Last known: ${described.title}` : described.title,
    tone: lastKnown ? 'attention' : described.tone,
    summary: described.summary,
    ...(notes.length > 0 && { notes }),
    hosts: scans.map((scan) => scan.row),
  };
}

/**
 * The optional plugin receiver, from the app's own `listening` flag. The app
 * reports no port, so none is shown; an unread flag is unknown, never off.
 */
export const pluginReceiver = (listening: boolean | undefined): SidebarProps['listener'] =>
  listening === undefined ? { status: 'unknown' } : { status: listening ? 'listening' : 'off' };
