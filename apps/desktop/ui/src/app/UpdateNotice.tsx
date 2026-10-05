import { useEffect, useSyncExternalStore } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { UpdateController, type UpdateState } from './update-controller';
import '../styles/update-notice.css';

const controller = new UpdateController({
  enabled: async () =>
    isTauri() && !import.meta.env.DEV && (await invoke<boolean>('updates_enabled')),
  check: async (options) => (await import('@tauri-apps/plugin-updater')).check(options),
  restart: () => invoke('restart_after_update'),
});

// The main renderer owns the controller. React StrictMode/unmounts only remove
// observers, never cancel or duplicate the renderer's background operation.
if (typeof window !== 'undefined')
  window.addEventListener('pagehide', () => controller.dispose(), { once: true });
if (import.meta.hot) import.meta.hot.dispose(() => controller.dispose());

export function UpdateNotice() {
  const state = useSyncExternalStore(controller.subscribe, controller.snapshot);
  useEffect(() => {
    controller.start();
  }, []);
  return (
    <UpdateNoticeView
      state={state}
      onRetry={() => void controller.retry()}
      onRestart={() => void controller.restartToUpdate()}
    />
  );
}

export function UpdateNoticeView({
  state,
  onRetry,
  onRestart,
}: {
  state: UpdateState;
  onRetry: () => void;
  onRestart: () => void;
}) {
  if (state.phase === 'idle' || state.phase === 'disabled') return null;
  if (state.phase === 'error')
    return (
      <div className="xt-sidebar-update" role="alert" title={state.message}>
        <span>{state.message}</span>
        <button type="button" className="xt-update-button" onClick={onRetry}>
          Retry update
        </button>
      </div>
    );
  return (
    <div className="xt-sidebar-update" role="status">
      {state.phase === 'checking' && 'Checking for updates…'}
      {state.phase === 'downloading' && (
        <>
          <span className="sr-only">Downloading update {state.version}…</span>
          <span aria-hidden="true">Downloading…</span>
          <progress
            aria-label="Update download"
            max={state.total || undefined}
            value={state.total ? Math.min(state.downloaded, state.total) : undefined}
          />
        </>
      )}
      {state.phase === 'ready' && (
        <>
          <span className="sr-only">
            Update {state.version} is ready. Your local data stays on this Mac.
          </span>
          <button
            type="button"
            className="xt-update-button"
            title={`Update ${state.version} is ready. Your local data stays on this Mac.`}
            onClick={onRestart}
          >
            Restart to update
          </button>
        </>
      )}
      {state.phase === 'installing' && 'Installing update. The app will restart…'}
      {state.phase === 'installed' && 'Update installed. Restarting…'}
    </div>
  );
}
