import { useLiveUpdates } from '../data/DataProvider';

/**
 * Says when data events are not heard: once an attempt has failed, and while
 * a manual reconnect is under way. A retry keeps one element and one
 * button across its states, so the button keeps focus from the click through
 * to the answer; while connecting it announces the attempt and a click does
 * nothing. No failure detail is shown.
 *
 * Each webview has its own runtime, so each surface that reads data (the
 * Shell and the tray popover) shows its own runtime's notice; the styling is
 * the surface's.
 */
export function LiveUpdatesNotice({
  className,
  buttonClassName,
}: {
  className: string;
  buttonClassName: string;
}) {
  const live = useLiveUpdates();
  const connecting = live.state === 'connecting';
  // The first attempt connects behind the runtime's startup fence.
  if (live.state === 'connected' || (connecting && live.attempt === 0)) return null;
  return (
    <div className={className}>
      {/* A fresh live region per state, so each change is announced. */}
      <span key={live.state} role={connecting ? 'status' : 'alert'}>
        {connecting
          ? 'Reconnecting live updates…'
          : live.attempt === 0
            ? 'Live updates are unavailable.'
            : 'Live updates are still unavailable.'}
      </span>{' '}
      <button
        type="button"
        className={buttonClassName}
        aria-disabled={connecting}
        onClick={connecting ? undefined : live.reconnect}
      >
        Reconnect
      </button>
    </div>
  );
}
