import type { SessionsSummary as Summary } from '../data/generated/SessionsSummary';
import { Button } from '../kit/Button';
import { count } from '../kit/format';
import { StatTile } from '../kit/StatTile';
import { agentTime } from './agent-duration';
import { continuous } from './metric-format';
import '../styles/sessions.css';

export const SUMMARY_SCOPE =
  'Counts cover all matching indexed sessions. Messages and agent working time cover the selected range. The table hides sessions still being checked and sub-sessions without a verified parent.';
export const SUMMARY_SCOPE_SHORT = 'Matching indexed sessions · messages/time in selected range';

export function SessionsSummary({
  summary,
  describedBy,
  pending,
  failed,
  refreshing,
  onRetry,
}: {
  summary: Summary | null | undefined;
  describedBy: string;
  pending: boolean;
  failed: boolean;
  refreshing: boolean;
  onRetry: () => void;
}) {
  const data = failed ? undefined : summary;
  const reason = failed
    ? 'Summary unavailable for these filters'
    : 'Session summary is still being read';
  const classification = data
    ? `${count(data.checking_sessions)} still being checked; ${count(data.unlinked_sub_sessions)} sub-sessions without a verified parent. These sessions can be hidden from the table.`
    : undefined;
  return (
    <section
      className="xt-sessions-summary"
      aria-label="Range summary"
      aria-describedby={describedBy}
      aria-busy={refreshing || undefined}
    >
      {failed && (
        <p className="xt-sessions-summary-error" role="status">
          <span>Session summary unavailable. Loaded sessions remain below.</span>
          <Button variant="outline" height={28} disabled={refreshing} onClick={onRetry}>
            Retry
          </Button>
        </p>
      )}
      <div className="xt-sessions-tiles">
        <StatTile
          label="Sessions"
          icon="lanes"
          ruleId="M-16"
          value={data?.main_sessions ?? null}
          format={count}
          reason={reason}
          unit="main"
          aside={
            data
              ? `${count(data.sub_sessions)} sub · ${count(data.checking_sessions)} checking`
              : undefined
          }
          definition="Main sessions whose checks have finished, plus known sub-sessions. Counts follow the table filters across all indexed matches, including sessions not loaded yet."
          tip={classification}
        />
        <StatTile
          label="Your input"
          icon="msg"
          ruleId="M-02"
          value={data?.human_messages ?? null}
          format={count}
          reason={pending || failed ? reason : 'Some messages have not been classified'}
          unit="messages"
          aside={
            data?.messages_per_main_session != null
              ? `${continuous(data.messages_per_main_session)} / main`
              : undefined
          }
          definition="Your messages in the selected range across all matching indexed sessions, including sub-sessions and sessions still being checked."
          tip={
            data?.messages_per_main_session == null && data
              ? 'Messages per main session is unavailable while checks are pending, there are no main sessions, or message counts are incomplete.'
              : 'Messages per main session includes input to matching sub-sessions.'
          }
        />
        <StatTile
          label="Agent working time"
          icon="clock"
          ruleId="M-05"
          value={data?.agent_ms ?? null}
          format={agentTime}
          reason={pending || failed ? reason : 'Agent working time could not be measured'}
          aside="Parallel sessions add together"
          definition="Active time in the selected range across all matching indexed sessions, including sub-sessions and sessions still being checked. Parallel sessions add together; this is not time saved."
        />
        <StatTile
          label="Sessions with PRs"
          icon="merge"
          ruleId="M-19"
          value={data?.sessions_with_prs ?? null}
          format={count}
          reason={reason}
          definition="Matching indexed sessions with at least one stored pull request link. Each session counts once, including inferred links. This does not count merged pull requests."
        />
      </div>
    </section>
  );
}
