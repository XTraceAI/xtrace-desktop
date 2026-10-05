import { useId, type ReactNode } from 'react';
import type { RuleId } from '../../kit/rules';
import { DefinitionInfo } from './DefinitionInfo';
import '../../styles/metrics.css';

/**
 * A compact titled card. Flat by default: one surface, with its children
 * following the title. `layered` draws it the way the kit SectionCard draws
 * Sessions — the title row on the page's canvas and the children inside an
 * inset surface panel, the kit's own `xt-section-panel`, so the two share
 * their tokens by construction. The panel is a flex column that passes the
 * card's height down, so only the lists inside it scroll. The header stays
 * this card's own wrapping row rather than the kit's fixed one, so the effort
 * controls never clip at the narrowest window.
 */
export function DashCard({
  title,
  rule,
  context,
  className = '',
  layered = false,
  actions,
  children,
}: {
  title: string;
  rule: RuleId;
  /** Read after the rule in the card's definition. */
  context?: string;
  className?: string;
  /** The title row on the canvas, the children in an inset surface panel. */
  layered?: boolean;
  /** Controls at the header's end, after the title and its definition. */
  actions?: ReactNode;
  children: ReactNode;
}) {
  const id = useId();
  return (
    <section
      className={`xt-dash-card ${className}`}
      data-layered={layered || undefined}
      aria-labelledby={id}
    >
      <header className="xt-dash-card-header">
        <h2 id={id}>{title}</h2>
        <DefinitionInfo ruleId={rule} name={title} context={context} />
        {actions && <span className="xt-dash-card-end">{actions}</span>}
      </header>
      {layered ? <div className="xt-section-panel">{children}</div> : children}
    </section>
  );
}
