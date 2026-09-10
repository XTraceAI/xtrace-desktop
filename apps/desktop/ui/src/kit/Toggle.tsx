import { Switch } from '@base-ui/react/switch';
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
    <Switch.Root
      nativeButton
      render={<button type="button" />}
      tabIndex={0}
      checked={checked}
      onCheckedChange={(checked) => onChange(checked)}
      aria-label={label}
      disabled={disabled}
      className="xt-toggle"
    >
      <span className="xt-toggle-track" aria-hidden="true">
        <Switch.Thumb />
      </span>
      {label}
      {trailing}
    </Switch.Root>
  );
}
