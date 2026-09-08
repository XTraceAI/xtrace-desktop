import type { ComponentProps } from 'react';
import '../styles/controls.css';

type Props = Omit<ComponentProps<'input'>, 'type' | 'value' | 'onChange' | 'aria-label'> & {
  label: string;
  value: string;
  onValueChange: (value: string) => void;
  shortcut?: string;
};

export function Search({ label, value, onValueChange, shortcut, className = '', ...props }: Props) {
  return (
    <label className={`xt-search ${className}`}>
      <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
        <circle cx="10" cy="10" r="6" />
        <path d="m15 15 5 5" />
      </svg>
      <input
        {...props}
        type="search"
        aria-label={label}
        value={value}
        onChange={(event) => onValueChange(event.target.value)}
      />
      {shortcut && <kbd aria-hidden="true">{shortcut}</kbd>}
    </label>
  );
}
