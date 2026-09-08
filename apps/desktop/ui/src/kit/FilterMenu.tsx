import { useId, useRef, useState } from 'react';
import { HostGlyph } from './HostGlyph';
import { Popover } from './Popover';
import '../styles/tables.css';

export interface HostOption {
  id: string;
  label: string;
  count?: number | null;
}
export function FilterMenu({
  options,
  selected,
  onChange,
  label = 'Filter hosts',
}: {
  options: readonly HostOption[];
  selected: readonly string[];
  onChange: (selected: string[]) => void;
  label?: string;
}) {
  const id = useId();
  const anchor = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  return (
    <span className="xt-filter-anchor">
      <button
        ref={anchor}
        className="xt-filter-trigger"
        type="button"
        tabIndex={0}
        aria-label={label}
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen(!open)}
      >
        <span className="xt-filter-glyphs" aria-hidden="true">
          {selected.map((host) => (
            <HostGlyph key={host} host={host} size={18} stacked />
          ))}
        </span>
        <span>{selected.length ? `${selected.length} selected` : 'All hosts'}</span>
        <span aria-hidden="true">⌄</span>
      </button>
      <Popover
        id={id}
        open={open}
        onOpenChange={setOpen}
        anchorRef={anchor}
        align="end"
        offset={6}
        className="xt-filter-menu"
      >
        <fieldset>
          <legend className="sr-only">{label}</legend>
          {options.length === 0 && <p className="xt-filter-empty">No hosts available</p>}
          {options.map((option) => (
            <label key={option.id} className="xt-filter-option">
              <input
                type="checkbox"
                tabIndex={0}
                checked={selected.includes(option.id)}
                onChange={() =>
                  onChange(
                    selected.includes(option.id)
                      ? selected.filter((key) => key !== option.id)
                      : [...selected, option.id],
                  )
                }
              />
              <span aria-hidden="true">
                <HostGlyph host={option.id} size={16} />
              </span>
              <span>{option.label}</span>{' '}
              <small>
                {option.count == null || !Number.isFinite(option.count)
                  ? '—'
                  : option.count.toLocaleString('en-US')}
              </small>
            </label>
          ))}
        </fieldset>
      </Popover>
    </span>
  );
}
