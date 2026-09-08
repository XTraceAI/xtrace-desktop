import { useRef, useState } from 'react';
import type { ControlTone } from './control-tone';
import '../styles/controls.css';

export type SegmentOption<T extends string> = {
  value: T;
  label: string;
  disabled?: boolean;
  tone?: ControlTone;
};

export function Segmented<T extends string>({
  label,
  options,
  value,
  onChange,
  disabled = false,
}: {
  label: string;
  options: readonly SegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
  disabled?: boolean;
}) {
  const [focused, setFocused] = useState<T | null>(null);
  const buttons = useRef(new Map<T, HTMLButtonElement>());
  const enabled = disabled ? [] : options.filter((option) => !option.disabled);
  const tabValue =
    enabled.find((option) => option.value === focused)?.value ??
    enabled.find((option) => option.value === value)?.value ??
    enabled[0]?.value;
  return (
    <div
      role="radiogroup"
      aria-label={label}
      aria-disabled={disabled || undefined}
      className="xt-segmented"
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setFocused(null);
      }}
    >
      {options.map((option) => (
        <button
          key={option.value}
          ref={(node) => {
            if (node) buttons.current.set(option.value, node);
            else buttons.current.delete(option.value);
          }}
          type="button"
          role="radio"
          aria-checked={option.value === value}
          disabled={disabled || option.disabled}
          tabIndex={option.value === tabValue ? 0 : -1}
          className="xt-segment xt-control-tone"
          data-tone={option.tone ?? 'meta'}
          onFocus={() => setFocused(option.value)}
          onClick={() => onChange(option.value)}
          onKeyDown={(event) => {
            const index = enabled.findIndex((item) => item.value === option.value);
            if (
              index < 0 ||
              !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End'].includes(
                event.key,
              )
            )
              return;
            event.preventDefault();
            const next =
              event.key === 'Home'
                ? 0
                : event.key === 'End'
                  ? enabled.length - 1
                  : (index +
                      (['ArrowLeft', 'ArrowUp'].includes(event.key) ? -1 : 1) +
                      enabled.length) %
                    enabled.length;
            const selected = enabled[next].value;
            buttons.current.get(selected)?.focus();
            onChange(selected);
          }}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}
