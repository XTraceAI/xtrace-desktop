import { Link } from 'react-router';
import type { SessionParentLink } from '../data/generated/SessionParentLink';
import { shortId } from './session-context';
import '../styles/subsession.css';

/**
 * A verified parent a row can show: the one the report named, and never the
 * row itself. An absent parent — none recorded, not indexed, ambiguous or
 * conflicting, all of which the report sends as `null` — leaves the row an
 * ordinary session. Nothing here infers a parent from a repository, a time
 * or what a session said.
 */
export function verifiedParent(row: {
  id: string;
  parent?: SessionParentLink | null;
}): SessionParentLink | null {
  const parent = row.parent ?? null;
  return parent && parent.session_id !== row.id ? parent : null;
}

/**
 * How a parent is named. `shown` is the name this view has for it right now —
 * its own row's, or a host title read for it where a row only names it — and
 * is `null` or `undefined` when there is none. Then the title the index saved
 * for it, else enough of its identity to tell it apart. No role or task is
 * claimed: the report states none.
 */
export const parentName = (parent: SessionParentLink, shown?: string | null) =>
  shown ?? parent.title ?? `Session ${shortId(parent.session_id)}`;

/**
 * The marker inside a sub-session's name cell: one link, to its parent's own
 * page, that says which session created this one. The row stays its own
 * session — its own link, metrics and place in the list — and the parent is
 * only named here, never added to the list or moved beside it.
 *
 * Its words give way before the parent's name does: a narrow cell shows the
 * mark and the name, and the link's name and tooltip still say it in full.
 */
export function ParentMarker({
  parent,
  name,
  to,
  className,
}: {
  parent: SessionParentLink;
  name: string;
  to: string;
  className: string;
}) {
  return (
    <Link
      className={`xt-subsession ${className}`}
      to={to}
      // The visible words lead the name, and the parent's whole identity
      // follows, so the link says exactly which session it opens.
      aria-label={`Sub-session of ${name}, open parent session ${parent.session_id}`}
      title={`Sub-session of ${name} · ${parent.session_id}`}
    >
      <span className="xt-subsession-mark" aria-hidden="true">
        ↳
      </span>
      <span className="xt-subsession-words">Sub-session of </span>
      <span className="xt-subsession-parent">{name}</span>
    </Link>
  );
}

/**
 * The disclosure for a group: a real button in the name cell, so the row keeps
 * its columns and its height. Its name says whose sub-sessions and how many
 * are counted (`noun` says which: returned by a report, loaded by a list);
 * `aria-expanded` says whether they are listed. It controls no single element
 * — the children are rows of the same list, mounted only while open — so it
 * names none with `aria-controls`.
 */
export function GroupToggle({
  row,
  name,
  noun,
  onToggle,
}: {
  row: { key: string; children: number; expanded: boolean };
  name: string;
  noun: string;
  onToggle: (key: string) => void;
}) {
  // Counted as the Dashboard's `plural` counts, without reading its module.
  const label = `${row.children.toLocaleString('en-US')} ${row.children === 1 ? noun : `${noun}s`} of ${name}`;
  return (
    <button
      type="button"
      className="xt-lane-toggle"
      // WebKit leaves a button out of the Tab order unless it is named here.
      tabIndex={0}
      aria-expanded={row.expanded}
      aria-label={label}
      title={`${row.expanded ? 'Hide' : 'Show'} ${label}`}
      onClick={() => onToggle(row.key)}
    >
      <span className="xt-lane-chevron" aria-hidden="true">
        ›
      </span>
      <span aria-hidden="true">{row.children.toLocaleString('en-US')}</span>
    </button>
  );
}
