import { Button } from './Button';
import '../styles/tables.css';

export function EmptyState({
  message,
  action,
}: {
  message: string;
  action?: { label: string; onClick: () => void };
}) {
  return (
    <div className="xt-empty-state">
      <p>{message}</p>
      {action && (
        <Button variant="outline" onClick={action.onClick}>
          {action.label}
        </Button>
      )}
    </div>
  );
}
export function LoadingRows({ rows = 4 }: { rows?: number }) {
  const count = Number.isFinite(rows) ? Math.max(1, Math.min(30, Math.floor(rows))) : 4;
  return (
    <div className="xt-loading-rows" role="status" aria-label="Loading rows">
      <span className="sr-only">Loading rows</span>
      {Array.from({ length: count }, (_, index) => (
        <div key={index} aria-hidden="true">
          <span />
          <span />
          <span />
        </div>
      ))}
    </div>
  );
}
