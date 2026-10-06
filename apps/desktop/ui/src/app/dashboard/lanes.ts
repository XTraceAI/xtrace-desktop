import type { DashboardLane } from '../../data/generated/DashboardLane';
import type { SessionChildCheck } from '../../data/generated/SessionChildCheck';

/**
 * The returned spans of one session, in the order the report sorted them
 * (most recent span first). `spans` holds every span the report returned for
 * this session: grouping never drops or merges a segment, so the drawn track
 * and the row's count describe exactly the same set.
 */
export interface SessionLane {
  /** Stable React key; not shown. */
  key: string;
  sessionId: string;
  host: string;
  spans: readonly DashboardLane[];
  /** Earliest start and latest end among the returned spans of this session. */
  firstMs: number;
  lastMs: number;
}

/**
 * One row per session instead of one per span.
 *
 * Sessions keep the report's own order: a session appears where its most
 * recent span did, so the list still reads newest first without this file
 * re-deriving any ordering. The key carries the host beside the session id
 * because a lane names both; ids are unique per host in the store, so in real
 * data this groups by the canonical session and never merges two hosts into
 * one row whose glyph could only name one of them.
 */
export function groupSessionLanes(lanes: readonly DashboardLane[]): SessionLane[] {
  const byKey = new Map<string, SessionLane>();
  for (const lane of lanes) {
    const key = `${lane.host}\u0000${lane.session_id}`;
    const found = byKey.get(key);
    if (!found) {
      byKey.set(key, {
        key,
        sessionId: lane.session_id,
        host: lane.host,
        spans: [lane],
        firstMs: lane.start_ms,
        lastMs: lane.end_ms,
      });
      continue;
    }
    found.spans = [...found.spans, lane];
    found.firstMs = Math.min(found.firstMs, lane.start_ms);
    found.lastMs = Math.max(found.lastMs, lane.end_ms);
  }
  return [...byKey.values()];
}

/**
 * What a context read says of one session, as far as whether a list shows it:
 * its host, its stored known-child bit and its display check.
 */
export interface ShownContext {
  host: string;
  known_child?: boolean | null;
  child_check?: SessionChildCheck | null;
}

/**
 * Whether a session is still being checked, by exact host and identity: the
 * supported checks of who created it have not finished, so a list does not
 * show it as a session of its own. No context for it, a context of another
 * host, or one without the display check reads as still being checked: what
 * is not known never shows a session.
 */
export const stillChecking = (found: ShownContext | undefined, host: string) =>
  !found || found.host !== host || found.child_check == null || found.child_check === 'checking';

/**
 * Whether a list leaves a session out: it is still being checked, or it is a
 * known sub-session whose creator is not verified (`verified` says whether its
 * parent link is). A checked main session, and a known sub-session under a
 * verified parent, are shown. A finished check never says a person created
 * the session.
 */
export const notListed = (found: ShownContext | undefined, host: string, verified: boolean) =>
  stillChecking(found, host) ||
  ((found!.child_check === 'child' || found!.known_child === true) && !verified);

/** The part of a verified parent link the grouping reads: which session, on which host. */
export interface LaneParent {
  session_id: string;
  host: string;
}

/**
 * What the grouping reads of a listed session: its key, exact identity and
 * host. A Dashboard lane is one; so is a Sessions row given the same key.
 */
export type ListedSession = Pick<SessionLane, 'key' | 'sessionId' | 'host'>;

/**
 * One row the compact table draws. A `session` row is a returned session's own
 * row, unchanged; `children` counts the returned sessions it verifiably
 * created, listed under it when `expanded`. An `absent` row stands for a
 * verified parent with no span in this report: it names that parent and counts
 * its returned children, and carries no lane or measurement of its own.
 * `members` is every session placed under the row at any depth — the rows
 * opening it, and every group inside it, would list — in that order; a row
 * with no children has none.
 */
export type LaneRow<P extends LaneParent = LaneParent, L extends ListedSession = SessionLane> =
  | {
      kind: 'session';
      key: string;
      lane: L;
      depth: number;
      children: number;
      expanded: boolean;
      members: readonly L[];
    }
  | {
      kind: 'absent';
      key: string;
      parent: P;
      depth: 0;
      children: number;
      expanded: boolean;
      members: readonly L[];
    };

export const sessionKey = (host: string, sessionId: string) => `${host}\u0000${sessionId}`;
/** The key of the row that stands for an absent parent with this session key. */
export const absentKey = (key: string) => `absent\u0000${key}`;

/**
 * The returned sessions the compact table may list, in the order given.
 *
 * `unresolved` says whether a session is left out ([`notListed`]): still being
 * checked, or a known sub-session whose creator is not verified. Such a
 * session is left out, and so is every returned session whose verified
 * parents lead to it, so grouping can never bring it back as an absent
 * parent's row. The walk follows parents only through returned sessions: a
 * parent with no span here is asked about itself, once, and its own parent is
 * not followed. A chain that comes back to where it started ends the walk.
 * Nothing else is left out.
 */
export function listedLanes<P extends LaneParent, L extends ListedSession = SessionLane>(
  sessions: readonly L[],
  parentOf: (lane: L) => P | null,
  unresolved: (session: LaneParent) => boolean,
): L[] {
  const byKey = new Map(sessions.map((lane) => [lane.key, lane]));
  const hidden = (lane: L) => {
    const seen = new Set<string>();
    for (let next: L | undefined = lane; next && !seen.has(next.key);) {
      seen.add(next.key);
      if (unresolved({ session_id: next.sessionId, host: next.host })) return true;
      const parent = parentOf(next);
      if (!parent) return false;
      next = byKey.get(sessionKey(parent.host, parent.session_id));
      if (!next) return unresolved(parent);
    }
    return false;
  };
  return sessions.filter((lane) => !hidden(lane));
}

/**
 * Sessions as the compact table lists them: each returned session with a
 * verified parent under that parent, collapsed unless its key is in `open`.
 *
 * Only the sessions given are placed, and each exactly once — under its parent
 * when the parent is one of them, else under one `absent` row for that parent,
 * so a collapsed child is always one disclosure away. A session with no
 * verified parent, or whose parents would lead back to itself, stays an
 * ordinary row. A group sits where its newest returned member does, and
 * members keep that same order among themselves; the report's own order
 * decides "newest", as it does for the ungrouped list. The union is flat —
 * only the rows to draw, each with its depth — so it fits a table of fixed-
 * height rows. `grouped` says whether any session was placed under another.
 */
export function laneRows<P extends LaneParent, L extends ListedSession = SessionLane>(
  sessions: readonly L[],
  parentOf: (lane: L) => P | null,
  open: ReadonlySet<string>,
): { rows: LaneRow<P, L>[]; grouped: boolean } {
  const byKey = new Map(sessions.map((lane) => [lane.key, lane]));
  const rank = new Map(sessions.map((lane, index) => [lane.key, index]));
  const parentKey = (lane: L) => {
    const parent = parentOf(lane);
    return parent ? sessionKey(parent.host, parent.session_id) : null;
  };
  // A chain of returned parents that comes back to where it started would
  // leave its members under nobody reachable, so they stay ordinary rows. A
  // session whose chain only runs into such a loop is not part of it: it goes
  // under its parent, which stays an ordinary row.
  const circular = (lane: L) => {
    const seen = new Set([lane.key]);
    for (let next = parentKey(lane); next !== null;) {
      if (next === lane.key) return true;
      if (seen.has(next)) return false;
      const found = byKey.get(next);
      if (!found) return false;
      seen.add(next);
      next = parentKey(found);
    }
    return false;
  };

  type Node = { row: 'session'; lane: L } | { row: 'absent'; key: string; parent: P };
  const children = new Map<string, L[]>();
  const roots: Node[] = [];
  const absent = new Map<string, Node>();
  for (const lane of sessions) {
    const parent = circular(lane) ? null : parentOf(lane);
    if (!parent) {
      roots.push({ row: 'session', lane });
      continue;
    }
    const key = sessionKey(parent.host, parent.session_id);
    children.set(key, [...(children.get(key) ?? []), lane]);
    if (!byKey.has(key) && !absent.has(key)) {
      const node: Node = { row: 'absent', key: absentKey(key), parent };
      absent.set(key, node);
      roots.push(node);
    }
  }
  const members = (node: Node) =>
    node.row === 'session'
      ? (children.get(node.lane.key) ?? [])
      : (children.get(sessionKey(node.parent.host, node.parent.session_id)) ?? []);
  const keyOf = (node: Node) => (node.row === 'session' ? node.lane.key : node.key);
  // The newest returned member of a node's whole group, itself included.
  const newest = new Map<string, number>();
  const placed = (node: Node): number => {
    const known = newest.get(keyOf(node));
    if (known !== undefined) return known;
    const own = node.row === 'session' ? rank.get(node.lane.key)! : Infinity;
    const value = Math.min(own, ...members(node).map((lane) => placed({ row: 'session', lane })));
    newest.set(keyOf(node), value);
    return value;
  };
  const byNewest = (list: Node[]) => [...list].sort((a, b) => placed(a) - placed(b));

  // Every session under a node at any depth, in the order opening every
  // group would list them. Each session is placed once, so this never repeats.
  const under = (node: Node): L[] =>
    byNewest(members(node).map((lane): Node => ({ row: 'session', lane }))).flatMap((child) =>
      child.row === 'session' ? [child.lane, ...under(child)] : [],
    );

  const rows: LaneRow<P, L>[] = [];
  const add = (node: Node, depth: number) => {
    const below = members(node).map((lane): Node => ({ row: 'session', lane }));
    const key = keyOf(node);
    const expanded = below.length > 0 && open.has(key);
    const all = under(node);
    rows.push(
      node.row === 'session'
        ? {
            kind: 'session',
            key,
            lane: node.lane,
            depth,
            children: below.length,
            expanded,
            members: all,
          }
        : {
            kind: 'absent',
            key,
            parent: node.parent,
            depth: 0,
            children: below.length,
            expanded,
            members: all,
          },
    );
    if (expanded) for (const child of byNewest(below)) add(child, depth + 1);
  };
  for (const root of byNewest(roots)) add(root, 0);
  return { rows, grouped: children.size > 0 };
}
