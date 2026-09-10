import { Radio } from '@base-ui/react/radio';
import { RadioGroup } from '@base-ui/react/radio-group';
import { Icon } from './icons';
import '../styles/topbar.css';

export type TimeRange = '7d' | '14d' | '30d';
export type SelectedRange = TimeRange | 'custom';
export type TopBarAction = 'share' | 'scan' | 'copy';
type RangeProps =
  | { showRange?: true; range: SelectedRange; onRange: (range: TimeRange) => void }
  | { showRange: false; range?: SelectedRange; onRange?: (range: TimeRange) => void };
export type TopBarProps = RangeProps & {
  crumb: string;
  subcrumb?: string;
  actionLabel?: string;
  actionIcon?: TopBarAction;
  onAction?: () => void;
  /** Opens the caller-owned date picker; range changes only after caller confirmation. */
  onCustomRange?: () => void;
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
  onCustomRange,
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
            <div className="xt-range">
              <RadioGroup<SelectedRange>
                className="xt-range-presets"
                aria-label="Date range"
                aria-orientation="horizontal"
                value={range}
                onValueChange={(value) => {
                  if (value !== 'custom') onRange?.(value);
                }}
              >
                {ranges.map((value) => (
                  <Radio.Root
                    key={value}
                    value={value}
                    nativeButton
                    render={<button type="button" />}
                    disabled={!onRange}
                  >
                    {value}
                  </Radio.Root>
                ))}
              </RadioGroup>
              <button
                className="xt-range-custom"
                type="button"
                tabIndex={0}
                aria-label="Custom range"
                title="Custom range"
                aria-pressed={range === 'custom'}
                disabled={!onCustomRange}
                onClick={onCustomRange}
              >
                <Icon name="calendar" size={12} />
              </button>
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
