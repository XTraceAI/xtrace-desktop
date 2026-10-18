import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import type { TelemetryControls } from '../../data/DataSource';
import type { TelemetryState } from '../../data/generated/TelemetryState';
import { queryKeys } from '../../data/query-client';

/**
 * The telemetry state and its two changes. Every change shows only the state
 * the app read back after saving, never the request; a failed change re-reads.
 */
export function useTelemetry(controls: TelemetryControls | undefined) {
  const client = useQueryClient();
  const state = useQuery({
    queryKey: queryKeys.telemetry,
    queryFn: () => controls!.read(),
    enabled: controls !== undefined,
  });
  const saved = (next: TelemetryState) => client.setQueryData(queryKeys.telemetry, next);
  const reread = () => void client.invalidateQueries({ queryKey: queryKeys.telemetry });
  const setEnabled = useMutation({
    mutationFn: (enabled: boolean) => controls!.setEnabled(enabled),
    onSuccess: saved,
    onError: reread,
  });
  const acknowledge = useMutation({
    mutationFn: () => controls!.acknowledgeDisclosure(),
    onSuccess: saved,
    onError: reread,
  });
  return { state, setEnabled, acknowledge };
}
