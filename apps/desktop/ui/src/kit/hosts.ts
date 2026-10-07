/**
 * The one name each agent host is shown by, wherever a host is named: the
 * Sessions filter, a glyph's tooltip, Welcome, Settings, the Dashboard and a
 * live status. A host this table does not know is shown as it was recorded.
 *
 * The account-usage widget is the one exception, on purpose: it reports the
 * Claude account's limits, not the Claude Code app, so it says "Claude".
 */
export const HOST_NAMES = {
  claude: 'Claude Code',
  codex: 'Codex',
  cursor: 'Cursor',
} as const;

export type KnownHost = keyof typeof HOST_NAMES;

export const isKnownHost = (host: string): host is KnownHost => Object.hasOwn(HOST_NAMES, host);

/** A host's display name, or the recorded host when it is not one this app knows. */
export const hostName = (host: string): string => (isKnownHost(host) ? HOST_NAMES[host] : host);

/** What a session with no recorded surface label is called, everywhere. */
export const UNKNOWN_SURFACE = 'unknown surface';

/** A host and one of its surfaces: `Claude Code · claude-cli`, `Codex · unknown surface`. */
export const surfaceLabel = (host: string, surface: string | null | undefined) =>
  `${hostName(host)} · ${surface ?? UNKNOWN_SURFACE}`;
