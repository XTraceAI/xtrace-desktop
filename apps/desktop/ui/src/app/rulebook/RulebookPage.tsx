import { Tooltip } from '@base-ui/react/tooltip';
import { useId, type ReactNode } from 'react';
import { Link, useLocation } from 'react-router';
import { StatePill } from '../../kit/Badge';
import { Button } from '../../kit/Button';
import { DataTable, MonoCell, NumCell, type Column } from '../../kit/DataTable';
import { count } from '../../kit/format';
import { SectionCard } from '../../kit/SectionCard';
import { StatTile } from '../../kit/StatTile';
import { useSurfaceTheme } from '../../theme/ThemeProvider';
import type { RuleActivityFire } from '../../data/generated/RuleActivityFire';
import type { RuleActivityGroup } from '../../data/generated/RuleActivityGroup';
import {
  bounded,
  describe,
  isLoaded,
  recordedFires,
  snapshotNote,
  UNREAD_NOTE,
  useRuleActivity,
  windowRows,
  type Loaded,
  type RuleActivity,
} from './rule-activity';
import '../../styles/rulebook.css';

/**
 * What the Rulebook can say on this device: the retained rows of one bounded
 * snapshot of the default local rulebook source over a fixed trailing 14 days,
 * read when the overview or the timeline opens and on an explicit Refresh.
 * Counts are recorded rows, never all rule activity, active rules or outcomes;
 * what the source does not record (blocked, adherence, judge cost) stays
 * unknown (—), never 0. Nothing here changes, enables or runs a rule, and no
 * row links to a rule or a session. No rule has a name here: a group is shown
 * by its position in the snapshot, its recorded IDs one hover or focus away.
 */
export type RulebookView = 'overview' | 'fires' | 'rule';

const UNAVAILABLE = 'unavailable';
const NOT_READ = 'Not read by this app';
const NOT_READ_META = 'not read by this app';
const SOURCE = 'Default local rulebook source';

export function RulebookPage({ view }: { view: RulebookView }) {
  if (view === 'rule') return <RuleDetail />;
  // Routes reuse this component across addresses; the key makes each view's
  // activation its own read, and leaving the view cancel it.
  return <RecordedActivity key={view} view={view} />;
}

function RecordedActivity({ view }: { view: 'overview' | 'fires' }) {
  const { activity, refresh, cancel } = useRuleActivity();
  const loaded = isLoaded(activity) ? activity.result : null;
  const rows = loaded && bounded(windowRows(loaded.counts.window_modes), loaded);
  const groups = loaded && bounded(loaded.observed_group_count, loaded);
  const shownFires = `Showing the latest ${loaded?.latest_fires.length} of ${rows} recorded rows in the window.`;
  const shownGroups = `Showing ${loaded?.observed_groups.length} of ${groups} observed groups; every group is counted above.`;
  // Said once per view, beside the numbers it explains.
  const numbering = loaded && loaded.observed_groups.length > 0 && (
    <p className="xt-rulebook-numbering">{NUMBERING}</p>
  );
  const read = <ReadBar activity={activity} loaded={loaded} refresh={refresh} cancel={cancel} />;
  const note = <p className="xt-rulebook-note">{loaded ? snapshotNote(loaded) : UNREAD_NOTE}</p>;
  const timeline = (
    <SectionCard
      title="Recorded timeline"
      meta={loaded ? `${rows} rows · newest first` : undefined}
      padding={0}
      footer={footer(
        loaded?.fires_truncated && <Truncated>{shownFires}</Truncated>,
        view === 'fires' && numbering,
      )}
    >
      {loaded ? <Timeline loaded={loaded} /> : <NotListed activity={activity} />}
    </SectionCard>
  );
  if (view === 'fires')
    return (
      <section className="xt-rulebook" data-view="fires">
        <BackLink />
        <Heading
          title="Recorded rule activity"
          subline="Recorded timeline · retained rows in one source snapshot · no row links to a session"
        />
        {read}
        {note}
        {timeline}
      </section>
    );
  const exact = loaded?.counts.precision === 'exact';
  // Without a snapshot the tile says what the read bar says, never "not read"
  // after a read has ended.
  const state = describe(activity);
  return (
    <section className="xt-rulebook" data-view="overview">
      <Heading
        title="Rulebook"
        subline="Recorded rule activity · retained rows in one source snapshot · unknown values show —, not zero"
      />
      {read}
      <div className="xt-rulebook-tiles">
        <StatTile
          label="Recorded fires"
          icon="bolt"
          ruleId="R-05"
          value={loaded && bounded(recordedFires(loaded.counts.window_modes), loaded)}
          reason={loaded ? undefined : state.lead}
          aside={loaded ? `14d · ${exact ? 'exact' : 'lower bound'}` : state.pill}
          tip="Rows recorded with mode advise or gate in the 14-day window of this source snapshot. A recorded mode is not an outcome."
        />
        <StatTile
          label="Blocked"
          icon="shield"
          ruleId="R-05"
          value={null}
          reason={NOT_READ}
          aside={UNAVAILABLE}
          tip="A gate row records how a rule was delivered, not whether an agent was stopped; no outcome is read."
        />
        <StatTile
          label="Adherence"
          icon="clock"
          ruleId="R-05"
          value={null}
          reason={NOT_READ}
          aside={UNAVAILABLE}
          tip="Needs every opportunity a rule had to fire, which is not recorded; a count of fires alone cannot give it."
        />
        <StatTile
          label="Judge overhead"
          icon="token"
          ruleId="R-08"
          value={null}
          reason={NOT_READ}
          aside={UNAVAILABLE}
          tip="No judge calls are run or recorded by this app."
        />
      </div>
      {note}
      <div className="xt-rulebook-row">
        <SectionCard
          title="Observed groups"
          meta={loaded ? `${groups} observed · latest first` : undefined}
          padding={0}
          footer={footer(
            loaded?.groups_truncated && <Truncated>{shownGroups}</Truncated>,
            numbering,
          )}
        >
          {loaded ? <Groups loaded={loaded} /> : <NotListed activity={activity} />}
        </SectionCard>
        {timeline}
      </div>
    </section>
  );
}

/**
 * The read's state, its source and window, and its two controls. Both stay in
 * place and are only marked unavailable, so focus stays where it was as a
 * read starts, stops and ends, and a second press of Refresh lands on a
 * Refresh that does nothing rather than on a Cancel.
 */
function ReadBar({
  activity,
  loaded,
  refresh,
  cancel,
}: {
  activity: RuleActivity;
  loaded: Loaded | null;
  refresh: () => void;
  cancel: () => void;
}) {
  const state = describe(activity);
  const reading = activity.phase === 'reading';
  const stopping = activity.phase === 'reading' && activity.stopping;
  // Announced politely, and only this line: never the rows.
  const status = loaded
    ? `${state.lead} · ${bounded(recordedFires(loaded.counts.window_modes), loaded)} recorded fires in ${bounded(loaded.observed_group_count, loaded)} observed groups`
    : state.lead;
  const lines = count(loaded?.counts.snapshot.lines);
  return (
    <div className="xt-rulebook-read" data-phase={activity.phase}>
      <div className="xt-rulebook-read-state">
        <StatePill tone={state.tone} height={20} outlined live={reading && !stopping}>
          {state.pill}
        </StatePill>
        <p className="xt-rulebook-lead" role="status">
          {status}
        </p>
        {activity.phase !== 'unsupported' && (
          <>
            <Button
              variant="outline"
              height={28}
              aria-disabled={reading || undefined}
              onClick={refresh}
            >
              Refresh
            </Button>
            <Button
              variant="ghost"
              height={28}
              aria-disabled={!reading || stopping || undefined}
              onClick={cancel}
            >
              {stopping ? 'Stopping…' : 'Cancel'}
            </Button>
          </>
        )}
      </div>
      {loaded ? (
        <p className="xt-rulebook-source">
          {SOURCE} · window <Instant value={loaded.window_start} /> to{' '}
          <Instant value={loaded.window_end} /> · read <Instant value={loaded.read_at} />
          {` · ${lines} lines scanned`}
        </p>
      ) : (
        <p className="xt-rulebook-detail">
          {SOURCE} · {state.detail}
        </p>
      )}
    </div>
  );
}

const local = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
  hourCycle: 'h23',
});
/** A row's instant in local time; the recorded value stays one hover away. */
const localTime = (value: string) => {
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? value : local.format(parsed);
};
/** A returned instant exactly as recorded. */
function Instant({ value }: { value: string }) {
  return <time dateTime={value}>{value}</time>;
}

function emptyMessage(loaded: Loaded) {
  return loaded.counts.precision === 'exact'
    ? '0 recorded rows in this source snapshot'
    : 'No rows found in the portion read';
}

/** An observed group's exact identity: its recorded rulebook (or none) and rule. */
type Pair = Pick<RuleActivityGroup, 'rulebook_id' | 'rule_id'>;
const pairKey = (pair: Pair) => JSON.stringify([pair.rulebook_id, pair.rule_id]);
/**
 * Each returned group's 1-based position in this snapshot, latest observed
 * first, by its exact pair. A position, never a rule's number or name: the
 * same rule can take another position after Refresh.
 */
const positions = (loaded: Loaded) =>
  new Map(loaded.observed_groups.map((group, index) => [pairKey(group), index + 1]));
const groupName = (position: number) => `Observed group ${position}`;
const NUMBERING =
  'Groups are numbered by position in this snapshot, not by rule; numbers may change on Refresh.';
const bookId = (pair: Pair) => pair.rulebook_id ?? 'unscoped';

/** A group's count reads as its snapshot's are: a lower bound shows ≥. */
const modeColumn = (
  loaded: Loaded,
  key: keyof RuleActivityGroup['modes'],
  header: string,
): Column<RuleActivityGroup> => ({
  key,
  header: <abbr title={key}>{header}</abbr>,
  // Wide enough for the reader's largest count as a lower bound, "≥ 10,000".
  width: '56px',
  align: 'right',
  render: (group) => (
    <NumCell value={group.modes[key]} format={(value) => bounded(value, loaded)} />
  ),
});
const groupColumns = (loaded: Loaded): readonly Column<RuleActivityGroup>[] => {
  const position = positions(loaded);
  return [
    {
      key: 'rule',
      header: 'Rule',
      width: 'minmax(0, 1fr)',
      render: (group) => {
        const at = position.get(pairKey(group))!;
        return (
          <RuleCell
            position={at}
            detail={`${group.rulebook_id === null ? 'unscoped · ' : ''}latest ${localTime(group.latest_observed)}`}
          >
            <Identity
              label={`ID of observed group ${at}`}
              lead={`${groupName(at)} is this group’s position in this snapshot, not a rule name.`}
              ids={[
                ['Rule ID', group.rule_id],
                ['Rulebook ID', bookId(group)],
              ]}
            />
          </RuleCell>
        );
      },
    },
    modeColumn(loaded, 'advise', 'Adv.'),
    modeColumn(loaded, 'gate', 'Gate'),
    modeColumn(loaded, 'suppressed', 'Supp.'),
    modeColumn(loaded, 'unrecognized', 'Unrec.'),
  ];
};

/** Observed (rulebook, rule) pairs; an unscoped pair stays its own group. */
function Groups({ loaded }: { loaded: Loaded }) {
  return (
    <DataTable
      label="Observed rule groups"
      columns={groupColumns(loaded)}
      rows={loaded.observed_groups}
      getRowKey={pairKey}
      stickyHeader
      emptyMessage={emptyMessage(loaded)}
    />
  );
}

const version = (fire: RuleActivityFire) =>
  fire.rule_version === null ? 'no version recorded' : `version ${fire.rule_version.value}`;
const recordedVersion = ({ rule_version: recorded }: RuleActivityFire) =>
  recorded === null
    ? 'none recorded'
    : recorded.kind === 'label'
      ? `${recorded.value} (label)`
      : recorded.value;
const context = (fire: RuleActivityFire) =>
  [fire.host, fire.tool, fire.hook_phase].filter(Boolean).join(' · ');
const timelineColumns = (loaded: Loaded): readonly Column<RuleActivityFire>[] => {
  const position = positions(loaded);
  const row = new Map(loaded.latest_fires.map((fire, index) => [fire.fire_id, index + 1]));
  return [
    {
      key: 'at',
      header: 'Recorded',
      width: '112px',
      render: (fire) => (
        <MonoCell>
          <time dateTime={fire.fired_at} title={fire.fired_at}>
            {localTime(fire.fired_at)}
          </time>
        </MonoCell>
      ),
    },
    {
      key: 'rule',
      header: 'Rule',
      width: 'minmax(0, 1.2fr)',
      render: (fire) => {
        // Only a returned group has a position; any other rule stays unnumbered.
        const at = position.get(pairKey(fire));
        return (
          <RuleCell position={at} detail={version(fire)}>
            <Identity
              label={`ID of timeline row ${row.get(fire.fire_id)}, ${at === undefined ? 'recorded rule' : `observed group ${at}`}`}
              lead={
                at === undefined
                  ? `This rule’s group is not among the ${loaded.observed_groups.length} returned, so it has no position here.`
                  : `${groupName(at)} is its group’s position in this snapshot, not a rule name.`
              }
              ids={[
                ['Rule ID', fire.rule_id],
                ['Rulebook ID', bookId(fire)],
                ['Fire ID', fire.fire_id],
                ['Recorded version', recordedVersion(fire)],
              ]}
            />
          </RuleCell>
        );
      },
    },
    {
      key: 'mode',
      header: 'Mode',
      width: '82px',
      // A recorded mode, not an outcome; an unrecognized value is not shown.
      render: (fire) => <MonoCell>{fire.mode.kind}</MonoCell>,
    },
    {
      key: 'context',
      header: <abbr title="host · tool · hook phase">Context</abbr>,
      width: 'minmax(0, 1fr)',
      render: (fire) => (
        <MonoCell muted={!context(fire)}>
          <span title={context(fire) || undefined}>{context(fire) || 'none recorded'}</span>
        </MonoCell>
      ),
    },
  ];
};

/** The latest returned rows, newest first; the same rows on both views. */
function Timeline({ loaded }: { loaded: Loaded }) {
  return (
    <DataTable
      label="Recorded timeline"
      columns={timelineColumns(loaded)}
      rows={loaded.latest_fires}
      getRowKey={(fire) => fire.fire_id}
      stickyHeader
      emptyMessage={emptyMessage(loaded)}
    />
  );
}

/**
 * A row's rule as the page can name it: its group's position in the snapshot,
 * or "Recorded rule" when its group was not returned, over its ID control and
 * one detail. In a narrow panel only the words give way; the number is never
 * cut.
 */
function RuleCell({
  position,
  detail,
  children,
}: {
  position: number | undefined;
  detail: string;
  children: ReactNode;
}) {
  const name = position === undefined ? 'Recorded rule' : groupName(position);
  return (
    <span className="xt-title-cell xt-rulebook-rule">
      <span title={name}>
        {position === undefined ? (
          <span>{name}</span>
        ) : (
          <>
            <span>Observed group</span> <span className="xt-rulebook-position">{position}</span>
          </>
        )}
      </span>
      <small>
        {children}
        <span title={detail}>{detail}</span>
      </small>
    </span>
  );
}

/**
 * A row's recorded IDs, exactly as returned, behind one small control so they
 * never crowd the row. The control is a real button named for its row and
 * described by the IDs themselves, so a screen reader hears them without
 * opening anything; hover and focus show them in a passive tooltip that Base
 * UI positions over the page, outside the list's scroll clip, and closes on
 * Escape with focus left in place. The IDs are inert text, wrapped when long.
 */
function Identity({
  label,
  lead,
  ids,
}: {
  /** Distinct per row. */
  label: string;
  lead: string;
  ids: readonly (readonly [string, string])[];
}) {
  const described = useId();
  const theme = useSurfaceTheme();
  return (
    <>
      <Tooltip.Root disableHoverablePopup>
        <Tooltip.Trigger
          render={
            <button
              type="button"
              tabIndex={0}
              className="xt-rulebook-identity"
              aria-label={label}
              aria-describedby={described}
            />
          }
          delay={0}
          closeOnClick={false}
        >
          ID
        </Tooltip.Trigger>
        <Tooltip.Portal data-theme={theme}>
          <Tooltip.Positioner
            className="xt-rule-positioner xt-rulebook-identity-positioner"
            positionMethod="fixed"
            side="bottom"
            align="start"
            sideOffset={6}
            collisionPadding={8}
          >
            <Tooltip.Popup role="tooltip" className="xt-rule-popover">
              <span className="xt-rulebook-identity-lead">{lead}</span>
              <dl className="xt-rulebook-identity-ids">
                {ids.map(([name, value]) => (
                  <div key={name}>
                    <dt>{name}</dt>
                    <dd>{value}</dd>
                  </div>
                ))}
              </dl>
            </Tooltip.Popup>
          </Tooltip.Positioner>
        </Tooltip.Portal>
      </Tooltip.Root>
      <span id={described} hidden>
        {ids.map(([name, value]) => `${name} ${value}`).join('. ')}
      </span>
    </>
  );
}

function NotListed({ activity }: { activity: RuleActivity }) {
  const text =
    activity.phase === 'unsupported'
      ? 'No rows are listed in this preview.'
      : activity.phase === 'reading'
        ? 'No rows are listed until a read completes.'
        : 'No rows are listed from this read.';
  return <p className="xt-rulebook-detail xt-rulebook-not-listed">{text}</p>;
}

function Truncated({ children }: { children: ReactNode }) {
  return <p className="xt-rulebook-truncated">{children}</p>;
}
/** A panel's footer lines, or no footer when it has none. */
function footer(first: ReactNode, second: ReactNode) {
  return first || second ? (
    <>
      {first}
      {second}
    </>
  ) : undefined;
}

/**
 * `/rulebook/:ruleId`: an address can name any rule, and no rule record is
 * matched against it, so nothing is read here. The address is repeated as the
 * one that was asked for, never presented as a rule that exists.
 */
function RuleDetail() {
  const { pathname } = useLocation();
  return (
    <section className="xt-rulebook" data-view="rule">
      <BackLink />
      <Heading title="Rule detail" subline="Unavailable locally · no rule record to match" />
      <SectionCard title="Rule" meta={NOT_READ_META}>
        <Unavailable lead="This rule cannot be shown">
          No local rule records are read in this build, so this address cannot be matched to a rule,
          its version, its mode or its fires. It is not evidence that the rule exists or was
          removed.
        </Unavailable>
        <p className="xt-rulebook-address">
          Requested address <span>{pathname}</span>
        </p>
      </SectionCard>
    </section>
  );
}

function Heading({ title, subline }: { title: string; subline: string }) {
  return (
    <div className="xt-rulebook-heading">
      <h1>{title}</h1>
      <p>{subline}</p>
    </div>
  );
}

function BackLink() {
  return (
    <Link className="xt-rulebook-back" to="/rulebook">
      ← Rulebook
    </Link>
  );
}

function Unavailable({ lead, children }: { lead: string; children: ReactNode }) {
  return (
    <div className="xt-rulebook-unavailable">
      <StatePill tone="meta" height={20} outlined>
        {UNAVAILABLE}
      </StatePill>
      <p className="xt-rulebook-lead">{lead}</p>
      <p className="xt-rulebook-detail">{children}</p>
    </div>
  );
}
