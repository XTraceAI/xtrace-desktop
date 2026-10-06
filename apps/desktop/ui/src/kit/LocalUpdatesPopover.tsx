import { useEffect, useRef, useState } from 'react';
import type { LocalUpdateControls } from '../data/DataSource';
import { Popover, PopoverClose, type PopoverProps } from './Popover';

export interface LocalUpdatesPopoverProps extends Pick<
  PopoverProps,
  'id' | 'open' | 'onOpenChange' | 'handle' | 'positionAnchor'
> {
  controls: LocalUpdateControls;
}

export function LocalUpdatesPopover({ controls, ...popover }: LocalUpdatesPopoverProps) {
  const mounted = useRef(false);
  const inFlight = useRef<{ controls: LocalUpdateControls } | null>(null);
  const [action, setAction] = useState<{
    controls: LocalUpdateControls;
    pending: boolean;
    error: boolean;
  } | null>(null);
  // A replaced capability gets its own action state, including an A → B → A change.
  if (action && action.controls !== controls) setAction(null);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      inFlight.current = null;
    };
  }, [controls]);
  const pending = action?.controls === controls && action.pending;
  const error = action?.controls === controls && action.error;
  async function viewReleases() {
    if (inFlight.current?.controls === controls) return;
    const request = { controls };
    inFlight.current = request;
    setAction({ controls, pending: true, error: false });
    let failed = false;
    try {
      await controls.viewPublicReleases();
    } catch {
      failed = true;
    } finally {
      if (inFlight.current === request) {
        inFlight.current = null;
        if (mounted.current) setAction({ controls, pending: false, error: failed });
      }
    }
  }
  return (
    <Popover
      {...popover}
      side="right"
      align="end"
      offset={14}
      className="xt-hub-popover xt-updates-popover"
      role="dialog"
      aria-labelledby={`${popover.id}-title`}
    >
      <div className="xt-hub-content">
        <div className="xt-hub-heading">
          <span id={`${popover.id}-title`}>Updates</span>
          <PopoverClose type="button" aria-label="Close Updates">
            esc
          </PopoverClose>
        </div>
        <p>
          This is a local development build. Public releases may omit local changes. Installing
          public updates from this app is disabled.
        </p>
        {error && (
          <p role="alert">Public releases could not be opened in your browser. Try again.</p>
        )}
        <button
          type="button"
          className="xt-hub-connect"
          disabled={pending}
          onClick={() => void viewReleases()}
        >
          View public releases
        </button>
        {pending && <p role="status">Opening your browser…</p>}
      </div>
    </Popover>
  );
}
