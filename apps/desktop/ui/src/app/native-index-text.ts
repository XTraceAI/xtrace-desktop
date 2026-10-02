import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';

export const phaseText = (status: NativeIndexStatus) => {
  const { phase } = status;
  switch (phase.phase) {
    case 'disabled':
      return `Disabled: ${phase.reason}`;
    case 'scanning':
      return `Scanning (${status.files_scanned} Claude transcripts read)`;
    case 'ready':
      return `Ready (${status.reconciles} reconciliations)`;
    case 'stopped':
      return 'Stopped';
  }
};
export const freshnessText = (status: NativeIndexStatus) => {
  const { freshness } = status;
  switch (freshness.freshness) {
    case 'unknown':
      return 'Not watching yet';
    case 'live':
      return 'Live: changes are reconciled as they happen';
    case 'degraded':
      return `Degraded: ${freshness.reason}`;
  }
};
