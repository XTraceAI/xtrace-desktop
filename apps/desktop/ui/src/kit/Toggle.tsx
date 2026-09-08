import type { ReactNode } from 'react';
import '../styles/controls.css';

export function Toggle({
  label,
  checked,
  onChange,
  disabled = false,
  trailing,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  trailing?: ReactNode;
}) {
  return (
    <button
      type="button"
      tabIndex={0}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      className="xt-toggle"
      onClick={() => onChange(!checked)}
    >
      <span className="xt-toggle-track" aria-hidden="true">
        <span />
      </span>
      {label}
      {trailing}
    </button>
  );
}
