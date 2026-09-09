import { Popover, type PopoverProps } from './Popover';
import '../styles/sidebar.css';

export interface HubPopoverProps extends Pick<
  PopoverProps,
  'id' | 'open' | 'onOpenChange' | 'anchorRef' | 'positionRef'
> {
  connected?: boolean;
  teamLabel?: string;
  onConnect?: () => void;
}

export function HubPopover({
  connected = false,
  teamLabel,
  onConnect,
  ...popover
}: HubPopoverProps) {
  return (
    <Popover
      {...popover}
      side="right"
      align="end"
      offset={14}
      className="xt-hub-popover"
      role="dialog"
      aria-labelledby={`${popover.id}-title`}
    >
      <div className="xt-hub-content">
        <div className="xt-hub-heading">
          <span>XTrace Hub</span>
          <button
            type="button"
            tabIndex={0}
            popoverTarget={popover.id}
            popoverTargetAction="hide"
            aria-label="Close XTrace Hub"
          >
            esc
          </button>
        </div>
        <h2 id={`${popover.id}-title`}>
          {connected
            ? `Connected${teamLabel ? ` to ${teamLabel}` : ' to XTrace Hub'}.`
            : 'Connect this desktop to your team.'}
        </h2>
        <ul>
          <li>
            <strong>Team sharing</strong>
            <span>Weekly cards, sessions and PR costs visible to your team.</span>
          </li>
          <li>
            <strong>Team governance</strong>
            <span>One rulebook per repo, review roles, manager view.</span>
          </li>
          <li>
            <strong>Team rule enforcement</strong>
            <span>Rules fire inside every teammate's agent, with a shared fires ledger.</span>
          </li>
        </ul>
        {!connected && (
          <button
            type="button"
            tabIndex={0}
            className="xt-hub-connect"
            disabled={!onConnect}
            onClick={onConnect}
          >
            Connect XTrace Hub
          </button>
        )}
      </div>
    </Popover>
  );
}
