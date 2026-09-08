import { Icon } from './icons';
import '../styles/topbar.css';

export type TimeRange = '7d' | '14d' | '30d';
export type TopBarAction = 'share' | 'scan' | 'copy';
type RangeProps =
  | { showRange?: true; range: TimeRange; onRange: (range: TimeRange) => void }
  | { showRange: false; range?: TimeRange; onRange?: (range: TimeRange) => void };
export type TopBarProps = RangeProps & {
  crumb: string;
  subcrumb?: string;
  actionLabel?: string;
  actionIcon?: TopBarAction;
  onAction?: () => void;
};

const ranges: TimeRange[] = ['7d', '14d', '30d'];

export function TopBar({
  crumb,
  subcrumb,
  showRange = true,
  range,
  onRange,
  actionLabel,
  actionIcon = 'share',
  onAction,
}: TopBarProps) {
  return (
    <header className="xt-topbar">
      <nav className="xt-topbar-breadcrumb" aria-label="Breadcrumb">
        <span className="xt-topbar-home">~</span>
        <span aria-hidden="true">/</span>
        <span
          className="xt-topbar-crumb"
          aria-current={subcrumb ? undefined : 'page'}
          title={crumb}
        >
          {crumb}
        </span>
        {subcrumb && (
          <>
            <span aria-hidden="true">/</span>
            <span className="xt-topbar-subcrumb" aria-current="page" title={subcrumb}>
              {subcrumb}
            </span>
          </>
        )}
      </nav>
      {(showRange || actionLabel?.trim()) && (
        <div className="xt-topbar-tools">
          {showRange && (
            <div className="xt-range" role="group" aria-label="Date range">
              {ranges.map((value) => (
                <button
                  key={value}
                  type="button"
                  tabIndex={0}
                  aria-pressed={range === value}
                  disabled={!onRange}
                  onClick={() => onRange?.(value)}
                >
                  {value}
                </button>
              ))}
            </div>
          )}
          {actionLabel?.trim() && (
            <button
              className="xt-topbar-action"
              type="button"
              tabIndex={0}
              disabled={!onAction}
              onClick={onAction}
              title={actionLabel}
            >
              <Icon name={actionIcon} size={13} />
              <span>{actionLabel}</span>
            </button>
          )}
        </div>
      )}
    </header>
  );
}
