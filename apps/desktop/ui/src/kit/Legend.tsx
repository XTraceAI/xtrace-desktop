import type { ReactNode } from 'react';
import type { ControlTone } from './control-tone';
import '../styles/tables.css';
import '../styles/controls.css';
export function Legend({
  items,
  dots = false,
}: {
  items: readonly { label: string; value?: ReactNode; tone: ControlTone }[];
  dots?: boolean;
}) {
  return (
    <ul className="xt-legend">
      {items.map((item, index) => (
        <li key={index}>
          <span
            aria-hidden="true"
            className="xt-control-tone"
            data-tone={item.tone}
            data-dot={dots || undefined}
          />
          {item.label}
          {item.value != null && <strong>{item.value}</strong>}
        </li>
      ))}
    </ul>
  );
}
