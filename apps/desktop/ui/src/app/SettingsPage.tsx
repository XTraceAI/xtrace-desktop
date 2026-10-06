import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { Fragment, useId, useRef, useState } from 'react';
import { discardStoredWords } from '../data/content-purges';
import { useData } from '../data/DataProvider';
import type {
  HumanBreakControls,
  RetentionControls,
  TypingSpeedControls,
} from '../data/DataSource';
import type { ContentPurge } from '../data/generated/ContentPurge';
import type { ContentRetention } from '../data/generated/ContentRetention';
import type { NativeIndexStatus } from '../data/generated/NativeIndexStatus';
import { events } from '../data/ipc-names';
import { queryKeys } from '../data/query-client';
import { eventPrefixes } from '../data/subscribe-invalidation';
import { Button } from '../kit/Button';
import { MetricCell } from '../kit/MetricCell';
import { Modal, ModalClose } from '../kit/Modal';
import { SectionCard } from '../kit/SectionCard';
import { Toggle } from '../kit/Toggle';
import { useTheme, type ThemePreference } from '../theme/ThemeProvider';
import { freshnessText, phaseText } from './native-index-text';
import { useAppInfo, useDbCounts, useNativeIndexStatus } from './useAppInfo';
import '../styles/settings.css';

export function SettingsPage() {
  const { source } = useData();
  const { preference, setPreference } = useTheme();
  return (
    <section className="xt-placeholder xt-settings">
      <h1>Settings</h1>
      <SectionCard title="Appearance">
        <label className="xt-appearance">
          Theme
          <select
            aria-label="Appearance"
            value={preference}
            onChange={(event) => setPreference(event.target.value as ThemePreference)}
          >
            <option value="system">System</option>
            <option value="dark">Dark</option>
            <option value="light">Light</option>
          </select>
        </label>
      </SectionCard>
      <TypingSpeed controls={source.typingSpeed} />
      <BreakLength controls={source.humanBreak} />
      {source.retention ? (
        <StoredContent controls={source.retention} />
      ) : (
        <SectionCard title="Stored content">
          <p className="xt-settings-note">
            Storage controls need the desktop app&apos;s database. They are unavailable in this
            preview.
          </p>
        </SectionCard>
      )}
      {source.kind !== 'preview' && <LocalDatabase />}
      {source.kind !== 'preview' && (
        <SectionCard title="Native index">
          <NativeIndex />
        </SectionCard>
      )}
      <SectionCard title="Not in this build">
        <p className="xt-settings-note">
          Telemetry, cloud sinks, the judge route and price-table selection have no settings yet.
          This build has no telemetry collector, so there is nothing to turn off.
        </p>
      </SectionCard>
    </section>
  );
}

/** Monkeytype's official test; its WPM counts five characters, spaces included, as a word. */
export const TYPING_TEST_URL = 'https://monkeytype.com/';
export const DEFAULT_WPM = 40;
const CHARACTERS_PER_WORD = 5;
const MIN_WPM = 1;
const MAX_WPM = 300;

/** A whole number of words per minute in bounds, or null; never a rounded guess. */
export function parseWpm(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d{1,3}$/.test(trimmed)) return null;
  const wpm = Number(trimmed);
  return wpm >= MIN_WPM && wpm <= MAX_WPM ? wpm : null;
}

const cpmText = (wpm: number) =>
  `${wpm} WPM (${(wpm * CHARACTERS_PER_WORD).toLocaleString()} characters per minute)`;

/** Opens the official test outside the app; a plain new-tab link in a browser preview. */
function TypingTestLink({ controls }: { controls?: TypingSpeedControls }) {
  const [failed, setFailed] = useState(false);
  return (
    <>
      <p className="xt-settings-note">
        <a
          className="xt-settings-link"
          href={TYPING_TEST_URL}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(event) => {
            if (!controls) return;
            // The app window never navigates: the system browser opens the page.
            event.preventDefault();
            setFailed(false);
            controls.openTest().catch(() => setFailed(true));
          }}
        >
          Test your typing speed on Monkeytype
        </a>
      </p>
      {failed && <p role="alert">The typing test could not be opened in your browser.</p>}
    </>
  );
}

/** The saved typing speed the typing estimate reads, with the official test beside it. */
function TypingSpeed({ controls }: { controls?: TypingSpeedControls }) {
  const hintId = useId();
  return (
    <SectionCard title="Typing speed">
      {controls ? (
        <TypingSpeedControl controls={controls} hintId={hintId} />
      ) : (
        <p className="xt-settings-note">
          The typing speed is saved in the desktop app&apos;s database. It cannot be changed in this
          preview, which uses {cpmText(DEFAULT_WPM)}.
        </p>
      )}
      <p id={hintId} className="xt-settings-note">
        Estimated Human time is counted message length divided by this speed, at{' '}
        {CHARACTERS_PER_WORD} characters per word, the same word Monkeytype counts: {DEFAULT_WPM}{' '}
        WPM, the default, is {DEFAULT_WPM * CHARACTERS_PER_WORD} characters per minute. The
        Dashboard uses it for every counted message in both the selected and the previous period.
        Counted length includes selected or pasted text. It is a rough estimate, not a measurement
        of your attention or of whether text was typed or pasted.
      </p>
      <TypingTestLink controls={controls} />
    </SectionCard>
  );
}

/**
 * How one save ended, from the setting as read afterwards: the requested
 * value, another value, or no successful read at all. Unknown keeps the
 * request, so a later check can settle it.
 */
type SaveOutcome = { state: 'saved' | 'failed' } | { state: 'unknown'; requested: number };

/** A write, or a direct read that settles an unknown save of `requested`. */
type SaveAction = { kind: 'save' | 'check'; requested: number };

/** One saved whole-number setting that every Dashboard range reads. */
interface SavedNumber {
  controls: { read(): Promise<number>; set(value: number): Promise<number> };
  queryKey: readonly unknown[];
  /** Shown in the saved summary, e.g. `40 WPM (200 characters per minute)`. */
  text: (value: number) => string;
  parse: (text: string) => number | null;
  inputLabel: string;
  maxLength: number;
  defaultValue: number;
  resetLabel: string;
  invalid: string;
  /** The setting in a sentence, e.g. `typing speed`. */
  noun: string;
  /** Its short name, e.g. `speed`. */
  short: string;
  /** The value as a request is stated, e.g. `40 WPM`. */
  requested: (value: number) => string;
  testId: string;
  className: string;
  summaryClassName: string;
}

/**
 * The saved value is shown until a save commits; an invalid edit or a failed
 * save changes nothing, and only a committed change re-reads the Dashboard.
 * While a save's outcome is unknown, the cached value is only the last
 * confirmed one: nothing is written and nothing re-reads it until the user
 * checks the saved value.
 */
function SavedNumberControl({ setting, hintId }: { setting: SavedNumber; hintId: string }) {
  const { controls, queryKey, noun, short } = setting;
  const client = useQueryClient();
  const inputId = useId();
  const errorId = useId();
  const [draft, setDraft] = useState<string | null>(null);
  const [outcome, setOutcome] = useState<SaveOutcome | null>(null);
  const unknown = outcome?.state === 'unknown' ? outcome : null;
  // One save or check at a time, even before the pending state renders.
  const busy = useRef(false);
  // Every cached range was computed at the old value; nothing else reads it.
  const refreshDashboards = () => void client.invalidateQueries({ queryKey: queryKeys.dashboards });
  const save = useMutation({
    // A write whose reply was lost may still have committed, so a failed write
    // is settled by reading the setting back once; never by the value cached before.
    mutationFn: async ({ kind, requested }: SaveAction): Promise<SaveOutcome> => {
      // No read started before this one may land afterwards.
      await client.cancelQueries({ queryKey, exact: true });
      const before = client.getQueryData<number>(queryKey);
      let now: number;
      try {
        now =
          kind === 'save'
            ? await controls.set(requested).catch(() => controls.read())
            : await controls.read();
      } catch {
        // It may have committed: every range is re-read at whatever was saved.
        if (kind === 'save') refreshDashboards();
        return { state: 'unknown', requested };
      }
      client.setQueryData(queryKey, now);
      // A check settles a save whose ranges were already re-read.
      if (kind === 'save' && now !== before) refreshDashboards();
      return { state: now === requested ? 'saved' : 'failed' };
    },
    onSuccess: (next) => {
      setOutcome(next);
      if (next.state === 'saved') setDraft(null);
    },
    onSettled: () => {
      busy.current = false;
    },
  });
  // Serialized with writes, and held while the saved value is unknown.
  const value = useQuery({
    queryKey,
    queryFn: () => controls.read(),
    enabled: !save.isPending && !unknown,
  });
  const saved = value.data;
  const text = draft ?? (saved === undefined ? '' : String(saved));
  const parsed = setting.parse(text);
  const invalid = draft !== null && parsed === null;
  const ready = saved !== undefined && !save.isPending && !unknown;
  const run = (action: SaveAction) => {
    busy.current = true;
    if (action.kind === 'save') setOutcome(null);
    save.mutate(action);
  };
  const submit = (next: number | null) => {
    if (busy.current || !ready || next === null || next === saved) return;
    run({ kind: 'save', requested: next });
  };
  const check = () => {
    if (busy.current || !unknown) return;
    run({ kind: 'check', requested: unknown.requested });
  };
  return (
    <>
      <dl className={setting.summaryClassName}>
        <dt>{unknown ? 'Last confirmed' : 'Saved'}</dt>
        <dd data-testid={setting.testId}>
          {saved !== undefined ? setting.text(saved) : value.isError ? 'Unavailable' : 'Reading…'}
        </dd>
      </dl>
      <form
        className={`xt-settings-control ${setting.className}`}
        onSubmit={(event) => {
          event.preventDefault();
          submit(parsed);
        }}
      >
        <label htmlFor={inputId}>{setting.inputLabel}</label>
        <input
          id={inputId}
          type="text"
          inputMode="numeric"
          autoComplete="off"
          spellCheck={false}
          maxLength={setting.maxLength}
          value={text}
          disabled={saved === undefined}
          aria-invalid={invalid}
          aria-describedby={invalid ? `${errorId} ${hintId}` : hintId}
          onChange={(event) => setDraft(event.target.value)}
        />
        <Button
          type="submit"
          variant="primary"
          height={28}
          disabled={!ready || parsed === null || parsed === saved}
        >
          {save.isPending && save.variables.kind === 'save' ? 'Saving…' : 'Save'}
        </Button>
        <Button
          variant="outline"
          height={28}
          disabled={!ready || saved === setting.defaultValue}
          onClick={() => {
            setDraft(null);
            submit(setting.defaultValue);
          }}
        >
          {setting.resetLabel}
        </Button>
        {unknown && (
          <Button variant="outline" height={28} disabled={save.isPending} onClick={check}>
            {save.isPending ? 'Checking…' : `Check saved ${short}`}
          </Button>
        )}
      </form>
      {invalid && (
        <p id={errorId} role="alert">
          {setting.invalid}
        </p>
      )}
      {value.isError && !unknown && <p role="alert">The {noun} could not be read.</p>}
      {unknown && (
        <p role="alert">
          The {noun} could not be read back, so whether {setting.requested(unknown.requested)} was
          saved is unknown. The {short} shown was last confirmed before this save. Check the saved{' '}
          {short} before changing it; the Dashboard will read the saved {short} again.
        </p>
      )}
      {outcome?.state === 'failed' && (
        <p role="alert">
          The {noun} could not be saved. The {short} shown is the saved one.
        </p>
      )}
    </>
  );
}

function TypingSpeedControl({
  controls,
  hintId,
}: {
  controls: TypingSpeedControls;
  hintId: string;
}) {
  return (
    <SavedNumberControl
      hintId={hintId}
      setting={{
        controls,
        queryKey: queryKeys.typingSpeed,
        text: cpmText,
        parse: parseWpm,
        inputLabel: 'Words per minute',
        maxLength: 3,
        defaultValue: DEFAULT_WPM,
        resetLabel: `Reset to ${DEFAULT_WPM} WPM`,
        invalid: `Enter a whole number from ${MIN_WPM} to ${MAX_WPM}.`,
        noun: 'typing speed',
        short: 'speed',
        requested: (wpm) => `${wpm} WPM`,
        testId: 'typing-speed-saved',
        className: 'xt-typing-speed',
        summaryClassName: 'xt-typing-speed-summary',
      }}
    />
  );
}

export const DEFAULT_BREAK_MINUTES = 60;
const MIN_BREAK_MINUTES = 5;
const MAX_BREAK_MINUTES = 240;

/** A whole number of minutes in bounds, or null; never a rounded guess. */
export function parseBreakMinutes(text: string): number | null {
  const trimmed = text.trim();
  if (!/^\d{1,3}$/.test(trimmed)) return null;
  const minutes = Number(trimmed);
  return minutes >= MIN_BREAK_MINUTES && minutes <= MAX_BREAK_MINUTES ? minutes : null;
}

/** The saved break length the Dashboard's "your hours" read. */
function BreakLength({ controls }: { controls?: HumanBreakControls }) {
  const hintId = useId();
  return (
    <SectionCard title="Your hours">
      {controls ? (
        <SavedNumberControl
          hintId={hintId}
          setting={{
            controls,
            queryKey: queryKeys.humanBreak,
            text: (minutes) => `${minutes} min`,
            parse: parseBreakMinutes,
            inputLabel: 'Break after (minutes)',
            maxLength: 3,
            defaultValue: DEFAULT_BREAK_MINUTES,
            resetLabel: `Reset to ${DEFAULT_BREAK_MINUTES} min`,
            invalid: `Enter a whole number from ${MIN_BREAK_MINUTES} to ${MAX_BREAK_MINUTES}.`,
            noun: 'break length',
            short: 'length',
            requested: (minutes) => `${minutes} min`,
            testId: 'human-break-saved',
            className: 'xt-typing-speed',
            summaryClassName: 'xt-typing-speed-summary',
          }}
        />
      ) : (
        <p className="xt-settings-note">
          The break length is saved in the desktop app&apos;s database. This preview uses{' '}
          {DEFAULT_BREAK_MINUTES} minutes.
        </p>
      )}
      <p id={hintId} className="xt-settings-note">
        Your hours on the Dashboard come from the times you sent messages to agents, in every
        conversation and tool. When two of your messages are at most this many minutes apart, the
        time between them counts. A longer gap, like lunch or sleep, counts as a break. A message on
        its own adds no time.
      </p>
    </SectionCard>
  );
}

const modeLabel: Record<ContentRetention, string> = {
  metadata_only: 'Metrics and indexing',
  full_content: 'Metrics, indexing and full-content archive',
};

function purgedRows(outcome: ContentPurge) {
  return outcome.tables.reduce((sum, table) => sum + table.rows, 0);
}

/** P-01 mode and the separate P-02 purge, read from and written to the app database. */
function StoredContent({ controls }: { controls: RetentionControls }) {
  const client = useQueryClient();
  const mode = useQuery({ queryKey: queryKeys.contentRetention, queryFn: () => controls.read() });
  const change = useMutation({
    mutationFn: (next: ContentRetention) => controls.set(next),
    // Only the mode the database reports after commit is shown; never the request.
    onSuccess: (saved) => client.setQueryData(queryKeys.contentRetention, saved),
    onError: () => void client.invalidateQueries({ queryKey: queryKeys.contentRetention }),
  });
  const saved = mode.data;
  return (
    <SectionCard title="Stored content">
      <dl className="xt-database-summary">
        <dt>Mode</dt>
        <dd data-testid="retention-mode">
          {saved ? modeLabel[saved] : mode.isError ? 'Unavailable' : 'Reading…'}
        </dd>
      </dl>
      <div className="xt-settings-control">
        <Toggle
          label="Archive full transcript content"
          checked={saved === 'full_content'}
          disabled={!saved || change.isPending}
          onChange={(checked) => change.mutate(checked ? 'full_content' : 'metadata_only')}
        />
      </div>
      <p className="xt-settings-note">
        Applies to future imports and enrichment. Existing saved content remains.
      </p>
      <p className="xt-settings-note">
        Off by default. Metrics and indexing keep counts, usage, tool names, identifiers and source
        provenance, and discard transcript text, tool results, session titles and tool input before
        saving. Turning the archive on saves that content for later imports.
      </p>
      <p className="xt-settings-note" data-testid="retention-previews">
        Either way, XTrace keeps a short preview (up to 280 characters) of each message you typed
        and of each Claude Code background-task summary, so the Dashboard can show them without
        reopening your session files. Delete stored content removes these previews too; messages
        indexed after that get new ones, and previews cannot be turned off.
      </p>
      {mode.isError && <p role="alert">The storage mode could not be read.</p>}
      {change.isError && (
        <p role="alert">The storage mode could not be changed. The mode shown is the saved one.</p>
      )}
      <PurgeContent controls={controls} archiving={saved === 'full_content'} />
    </SectionCard>
  );
}

function PurgeContent({
  controls,
  archiving,
}: {
  controls: RetentionControls;
  archiving: boolean;
}) {
  const client = useQueryClient();
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const title = useId();
  const purge = useMutation({
    mutationFn: () => controls.purge(),
    onSuccess: (outcome) => {
      setOpen(false);
      // The outcome is committed; refresh what could have shown the cleared content.
      if (outcome.invalidate_content) {
        discardStoredWords(client);
        for (const prefix of eventPrefixes[events.contentPurged])
          void client.invalidateQueries({ queryKey: [prefix] });
      }
    },
  });
  const done = purge.data;
  return (
    <div className="xt-settings-purge">
      <Button
        ref={trigger}
        variant="outline"
        height={28}
        disabled={purge.isPending}
        onClick={() => {
          purge.reset();
          setOpen(true);
        }}
      >
        Delete stored content
      </Button>
      <p className="xt-settings-note">
        Removes content already saved in the XTrace database. Original host transcripts are never
        modified.
      </p>
      {done && (
        <p role="status">
          {done.invalidate_content
            ? `Deleted stored content from ${purgedRows(done).toLocaleString()} ${purgedRows(done) === 1 ? 'row' : 'rows'}.`
            : 'There was no stored content to delete.'}
        </p>
      )}
      <Modal
        open={open}
        onOpenChange={(next) => {
          if (!purge.isPending) setOpen(next);
        }}
        returnFocusRef={trigger}
        aria-labelledby={title}
        className="xt-settings-modal"
      >
        <h2 id={title}>Delete stored content?</h2>
        <p>
          This clears saved transcript text, tool results, session titles, tool input and the short
          message previews from the XTrace database. Counts, usage, tool names, identifiers and
          source provenance stay. Original host transcripts and other files are not touched. Deleted
          text is not securely erased from database free space or backups. This cannot be undone.
        </p>
        <p>Messages indexed after the delete get new previews; previews cannot be turned off.</p>
        {archiving && <p>Full-content archive is on, so later imports can save content again.</p>}
        {purge.isError && <p role="alert">Stored content could not be deleted.</p>}
        <div className="xt-settings-actions">
          <ModalClose
            className="xt-button"
            data-variant="outline"
            style={{ height: 28 }}
            disabled={purge.isPending}
          >
            Cancel
          </ModalClose>
          <Button
            variant="primary"
            height={28}
            disabled={purge.isPending}
            onClick={() => purge.mutate()}
          >
            {purge.isPending ? 'Deleting…' : 'Delete stored content'}
          </Button>
        </div>
      </Modal>
    </div>
  );
}

function LocalDatabase() {
  const info = useAppInfo();
  const counts = useDbCounts();
  return (
    <SectionCard
      title="Local database"
      right={
        <Button
          variant="outline"
          height={28}
          disabled={info.isFetching || counts.isFetching}
          onClick={() => {
            void info.refetch();
            void counts.refetch();
          }}
        >
          Refresh
        </Button>
      }
    >
      <dl className="xt-database-summary">
        <dt>Data folder</dt>
        <dd>{info.data?.data_dir ?? 'Unavailable'}</dd>
        <dt>Schema version</dt>
        <dd>
          <MetricCell value={info.data?.schema_version} />
        </dd>
        <dt>Sessions</dt>
        <dd>
          <MetricCell value={counts.data?.sessions} reason="Database counts are unavailable" />
        </dd>
        <dt>Records</dt>
        <dd>
          <MetricCell value={counts.data?.records} reason="Database counts are unavailable" />
        </dd>
        <dt>Usage rows</dt>
        <dd>
          <MetricCell value={counts.data?.usage} reason="Database counts are unavailable" />
        </dd>
      </dl>
      {counts.isError && <p role="alert">Database counts could not be loaded.</p>}
    </SectionCard>
  );
}

function NativeIndex() {
  const index = useNativeIndexStatus();
  return index.data ? (
    <>
      <NativeIndexSummary status={index.data} />
      <HostScanScope hosts={index.data.hosts} />
    </>
  ) : index.isError ? (
    <p role="alert">Native index status could not be loaded.</p>
  ) : (
    <p role="status">Reading native index status…</p>
  );
}

const hostText = (host: NativeIndexStatus['hosts'][number]) => {
  const parts = [
    host.state,
    `${host.sessions_imported} imported, ${host.sessions_partial} partial, ${host.sessions_skipped} skipped, ${host.records_new} new records, ${host.records_enriched} enriched`,
  ];
  if (host.diagnostics > 0)
    parts.push(`${host.diagnostics} ${host.diagnostics === 1 ? 'diagnostic' : 'diagnostics'}`);
  if (host.detail) parts.push(host.detail);
  return parts.join(' · ');
};

/**
 * What a host's state covers: the scan of the sources the index reads. The
 * Cursor reader reads transcript files, never the Cursor IDE's own database,
 * so a complete Cursor scan says nothing about conversations kept only there.
 */
function HostScanScope({ hosts }: { hosts: NativeIndexStatus['hosts'] }) {
  if (hosts.length === 0) return null;
  return (
    <p className="xt-settings-note" data-testid="native-index-scope">
      Complete and incomplete describe each host&apos;s last scan of the sources this index reads,
      not all of that host&apos;s history.
      {hosts.some((host) => host.host === 'cursor') &&
        " Conversations kept only in the Cursor IDE's database are not read, so they are not in Cursor's counts."}
    </p>
  );
}

/** The typed status the app publishes; every field is shown as reported, never invented. */
function NativeIndexSummary({ status }: { status: NativeIndexStatus }) {
  return (
    <dl className="xt-native-index-summary" data-testid="native-index">
      <dt>State</dt>
      <dd>{phaseText(status)}</dd>
      <dt>Freshness</dt>
      <dd>{freshnessText(status)}</dd>
      <dt>Python</dt>
      <dd>
        {status.python.state === 'available'
          ? status.python.path
          : status.python.state === 'resolving'
            ? 'Resolving…'
            : `Unavailable: ${status.python.reason}`}
      </dd>
      <dt>Readers</dt>
      <dd>
        {status.readers.state === 'verified'
          ? `Bundled memhub ${status.readers.plugin_version} at ${status.readers.commit.slice(0, 12)}`
          : `Unavailable: ${status.readers.reason}`}
      </dd>
      {status.hosts.map((host) => (
        <Fragment key={host.host}>
          <dt>{host.host}</dt>
          <dd>
            <span>{hostText(host)}</span>
            <SkippedConversations host={host} />
          </dd>
        </Fragment>
      ))}
    </dl>
  );
}

const skipReasonText = {
  unreadable: 'Conversation file could not be read.',
  invalid_transcript: 'Conversation data has an invalid format.',
  invalid_header: 'Conversation header has an invalid format.',
  identity_conflict: 'Conversation identity is missing or conflicts with the index.',
  invalid_surface: 'Conversation source labels disagree or are invalid.',
  checkpoint_failed: 'Scan progress could not be read or saved.',
  write_failed: 'Conversation could not be saved to the index.',
  reader_incomplete: 'Reader stopped before completing the conversation.',
  unknown: 'Reason unavailable.',
};

function SkippedConversations({ host }: { host: NativeIndexStatus['hosts'][number] }) {
  if (host.sessions_skipped === 0) return null;
  return (
    <details className="xt-skipped-conversations">
      <summary>Skipped conversation details ({host.host})</summary>
      <p className="xt-settings-note">
        These are skips reported by this scan, including earlier outcomes it retained. A skip can
        still have records in the index. IDs and reasons are shown only when available.
      </p>
      <ul>
        {host.skipped_conversations.map((conversation, index) => (
          <li key={index}>
            {conversation.conversation_id ?? 'ID unavailable'} —{' '}
            {skipReasonText[conversation.reason]}
          </li>
        ))}
      </ul>
      {host.skipped_conversations_omitted > 0 && (
        <p>
          Showing {host.skipped_conversations.length} of {host.sessions_skipped} skipped
          conversations. {host.skipped_conversations_omitted} more omitted.
        </p>
      )}
    </details>
  );
}
