import { useId, type CSSProperties, type ReactNode } from 'react';
import { RuleChip } from './RuleChip';
import type { RuleId } from './rules';
import '../styles/metrics.css';

export function SectionCard({
  title,
  ruleId,
  meta,
  right,
  headerHeight = 40,
  padding = '12px 14px 10px',
  children,
  footer,
}: {
  title: string;
  ruleId?: RuleId;
  meta?: string;
  right?: ReactNode;
  headerHeight?: 36 | 40;
  padding?: CSSProperties['padding'];
  children: ReactNode;
  footer?: ReactNode;
}) {
  const id = useId();
  return (
    <section className="xt-section-card" aria-labelledby={id}>
      <header className="xt-section-header" style={{ height: headerHeight }}>
        <h2 id={id} title={title}>
          {title}
        </h2>
        {ruleId && <RuleChip ruleId={ruleId} />}
        {meta && (
          <span className="xt-section-meta" title={meta}>
            {meta}
          </span>
        )}
        {right && <span className="xt-section-right">{right}</span>}
      </header>
      <div className="xt-section-panel" style={{ padding }}>
        {children}
        {footer && <footer className="xt-section-footer">{footer}</footer>}
      </div>
    </section>
  );
}
