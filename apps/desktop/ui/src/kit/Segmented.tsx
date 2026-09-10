import { Radio } from '@base-ui/react/radio';
import { RadioGroup } from '@base-ui/react/radio-group';
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
  // Invalid or disabled selections must not strand the group's Tab entry.
  // Rebuild Base UI's item registry only when the available choices change.
  const selected = options.some((option) => option.value === value && !option.disabled)
    ? value
    : null;
  const choicesKey = JSON.stringify(options.map((option) => [option.value, !!option.disabled]));
  return (
    <RadioGroup<T | null>
      key={choicesKey}
      aria-label={label}
      aria-orientation="horizontal"
      disabled={disabled}
      value={selected}
      onValueChange={(value) => {
        if (value !== null) onChange(value);
      }}
      className="xt-segmented"
    >
      {options.map((option) => (
        <Radio.Root
          key={option.value}
          value={option.value}
          nativeButton
          render={<button type="button" />}
          disabled={option.disabled}
          className="xt-segment xt-control-tone"
          data-tone={option.tone ?? 'meta'}
        >
          {option.label}
        </Radio.Root>
      ))}
    </RadioGroup>
  );
}
