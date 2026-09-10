import type { ReactNode } from 'react';
import type { ControlTone } from './control-tone';
import '../styles/controls.css';

const kinds = {
  skill: 'accent',
  agent: 'accent',
  mcp: 'info',
  hook: 'success',
  plugin: 'success',
  cmd: 'warning',
  feature: 'accent',
  bug: 'danger',
  chore: 'meta',
} as const;

export function KindBadge({ kind, tone }: { kind: keyof typeof kinds; tone?: ControlTone }) {
  return (
    <span className="xt-kind xt-control-tone" data-tone={tone ?? kinds[kind]}>
      {kind}
    </span>
  );
}

export function EvidenceDot({ evidence }: { evidence: 'exact' | 'sha' | 'inferred' }) {
  const tone = { exact: 'success', sha: 'info', inferred: 'meta' } as const;
  return (
    <span
      role="img"
      aria-label={`${evidence} evidence`}
      title={`${evidence} evidence`}
      className="xt-evidence xt-control-tone"
      data-tone={tone[evidence]}
    />
  );
}

export function StatePill({
  children,
  tone = 'meta',
  height = 24,
  outlined = false,
  live = false,
}: {
  children: ReactNode;
  tone?: ControlTone;
  height?: 20 | 24 | 26;
  outlined?: boolean;
  live?: boolean;
}) {
  return (
    <span
      className="xt-state xt-control-tone"
      data-tone={tone}
      data-outlined={outlined}
      data-live={live}
      style={{ height, paddingInline: height === 20 ? 8 : height === 26 ? 10 : 9 }}
    >
      <span className="xt-state-dot" aria-hidden="true" />
      {children}
    </span>
  );
}
