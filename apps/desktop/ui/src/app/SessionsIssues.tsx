import { useId } from 'react';
import { Link } from 'react-router';
import type { DashboardUntimed } from '../data/generated/DashboardUntimed';
import type { TimeRange } from '../kit/TopBar';
import { IssueFlag } from './dashboard/IssueFlag';
import { plural } from './dashboard/present';
import { useDashboardReport } from './dashboard/range';
import { UNTIMED_SUMMARY, UntimedBody } from './dashboard/UntimedNotice';
import '../styles/sessions.css';

/** A known index warning: one short state and the sentence that explains it. */
export type IndexIssue = { state: string; text: string };

/**
 * What limits this page's history, stated as one small triangle at the end of
 * the heading's controls, as the Dashboard's panels state theirs: a known
 * index warning, and untimed history a current range report has counted. It
 * is drawn only when there is one, so it never marks a healthy or unread
 * index. Hovering or focusing it reads the states and the count; pressing it
 * opens both explanations, with the way to Settings.
 */
export function SessionsIssueFlag({
  index,
  untimed,
}: {
  index: IndexIssue | null;
  untimed?: DashboardUntimed;
}) {
  const titleId = useId();
  const counted = untimed && untimed.records > 0 ? untimed : null;
  if (!index && !counted) return null;
  const records = counted && plural(counted.records, 'untimed record');
  const title = !index
    ? 'Untimed history'
    : counted
      ? 'Index status and untimed history'
      : 'Index status';
  return (
    <IssueFlag
      label={`History limits: ${[index?.state, records].filter(Boolean).join(', ')}`}
      title={title}
      testId="sessions-issues"
      gist={
        <>
          {index && (
            <span>
              <strong>{index.state}</strong>: {index.text}
            </span>
          )}
          {counted && (
            <span>
              <strong>{records}</strong>: {UNTIMED_SUMMARY}, in all indexed history.
            </span>
          )}
        </>
      }
    >
      {index && (
        <section
          className="xt-sessions-issue"
          aria-labelledby={`${titleId}-index`}
          data-testid="sessions-index-status"
        >
          <h3 id={`${titleId}-index`} className="xt-dash-label">
            {index.state}
          </h3>
          <p className="xt-dash-note">
            {index.text} <Link to="/settings">View indexing status</Link>
          </p>
        </section>
      )}
      {counted && (
        <section className="xt-sessions-issue" aria-labelledby={`${titleId}-untimed`}>
          <h3 id={`${titleId}-untimed`} className="xt-dash-label">
            Untimed history
          </h3>
          <UntimedBody untimed={counted} />
        </section>
      )}
    </IssueFlag>
  );
}

/**
 * The flag beside a range summary: the untimed count comes from the same
 * cached range report the tiles read, and only while it is current. Until the
 * report answers, while a refresh of it is being read and once a refresh
 * fails, no count is shown: the cache keeps the previous answer through a
 * refresh, and that retained count would be presented as confirmed when it is
 * not. A known index warning does not depend on the report and stays.
 */
export function RangeIssueFlag({ index, range }: { index: IndexIssue | null; range: TimeRange }) {
  const report = useDashboardReport(range);
  return (
    <SessionsIssueFlag
      index={index}
      untimed={report.isError || report.isFetching ? undefined : report.data?.untimed_history}
    />
  );
}
