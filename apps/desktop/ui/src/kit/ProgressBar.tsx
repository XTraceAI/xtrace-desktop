import type { ControlTone } from './control-tone';
import '../styles/controls.css';

type Segment = { label: string; value: number; tone: ControlTone };
type Props = { label: string; height?: 3 | 4 | 6 } & (
  | { value: number | null; tone?: ControlTone; segments?: never }
  | { segments: readonly Segment[]; value?: never; tone?: never }
);
const clamp = (value: number) => Math.min(100, Math.max(0, value));

export function ProgressBar({ label, height = 4, value, tone = 'accent', segments }: Props) {
  const measured = segments
    ? segments.length > 0 && segments.every((item) => Number.isFinite(item.value))
    : typeof value === 'number' && Number.isFinite(value);
  const parts = measured
    ? (segments ?? [{ label, value: value!, tone }]).map((part) => ({
        ...part,
        value: Math.max(0, part.value),
      }))
    : [];
  const sum = parts.reduce((total, part) => total + part.value, 0);
  // Normalize oversized segments together, preserving their relative proportions.
  const largest = parts.reduce((max, part) => Math.max(max, part.value), 0);
  const relativeSum = largest ? parts.reduce((total, part) => total + part.value / largest, 0) : 0;
  const rendered = parts.map((part) => ({
    ...part,
    width: sum > 100 ? (part.value / largest / relativeSum) * 100 : part.value,
  }));
  return (
    <div
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={measured ? clamp(sum) : undefined}
      aria-valuetext={
        !measured
          ? 'Unmeasured'
          : segments
            ? rendered.map((part) => `${part.label}: ${Number(part.width.toFixed(1))}%`).join('; ')
            : undefined
      }
      className="xt-progress"
      data-segmented={!!segments}
      style={{ height }}
    >
      {rendered.map((part, index) => (
        <span
          key={index}
          aria-hidden="true"
          title={part.label}
          className="xt-control-tone"
          data-tone={part.tone}
          style={{ width: `${part.width}%` }}
        />
      ))}
    </div>
  );
}
