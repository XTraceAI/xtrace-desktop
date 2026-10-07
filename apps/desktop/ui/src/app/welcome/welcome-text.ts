import type { NativeHostStatus } from '../../data/generated/NativeHostStatus';
import type { NativeIndexStatus } from '../../data/generated/NativeIndexStatus';
import type { ControlTone } from '../../kit/control-tone';

/** A host's display name is the app's one name for it (the kit's `hostName`). */
export { hostName } from '../../kit/hosts';

/** Hosts whose last scan needs attention, as the app decides it (`needs_attention`). */
export const attentionCount = (hosts: readonly NativeHostStatus[]) =>
  hosts.filter((host) => host.needs_attention).length;

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
      return status.needs_attention
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
