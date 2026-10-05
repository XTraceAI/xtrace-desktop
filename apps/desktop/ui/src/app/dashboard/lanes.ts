import type { DashboardLane } from '../../data/generated/DashboardLane';

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

/** The part of a verified parent link the grouping reads: which session, on which host. */
export interface LaneParent {
  session_id: string;
  host: string;
}

/**
 * One row the compact table draws. A `session` row is a returned session's own
 * row, unchanged; `children` counts the returned sessions it verifiably
 * created, listed under it when `expanded`. An `absent` row stands for a
 * verified parent with no span in this report: it names that parent and counts
 * its returned children, and carries no lane or measurement of its own.
 */
export type LaneRow<P extends LaneParent = LaneParent> =
  | {
      kind: 'session';
      key: string;
      lane: SessionLane;
      depth: number;
      children: number;
      expanded: boolean;
    }
  | {
      kind: 'absent';
      key: string;
      parent: P;
      depth: 0;
      children: number;
      expanded: boolean;
    };

const sessionKey = (host: string, sessionId: string) => `${host}\u0000${sessionId}`;

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
export function laneRows<P extends LaneParent>(
  sessions: readonly SessionLane[],
  parentOf: (lane: SessionLane) => P | null,
  open: ReadonlySet<string>,
): { rows: LaneRow<P>[]; grouped: boolean } {
  const byKey = new Map(sessions.map((lane) => [lane.key, lane]));
  const rank = new Map(sessions.map((lane, index) => [lane.key, index]));
  const parentKey = (lane: SessionLane) => {
    const parent = parentOf(lane);
    return parent ? sessionKey(parent.host, parent.session_id) : null;
  };
  // A chain of returned parents that comes back to where it started would
  // leave its members under nobody reachable, so they stay ordinary rows. A
  // session whose chain only runs into such a loop is not part of it: it goes
  // under its parent, which stays an ordinary row.
  const circular = (lane: SessionLane) => {
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

  type Node = { row: 'session'; lane: SessionLane } | { row: 'absent'; key: string; parent: P };
  const children = new Map<string, SessionLane[]>();
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
      const node: Node = { row: 'absent', key: `absent\u0000${key}`, parent };
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

  const rows: LaneRow<P>[] = [];
  const add = (node: Node, depth: number) => {
    const below = members(node).map((lane): Node => ({ row: 'session', lane }));
    const key = keyOf(node);
    const expanded = below.length > 0 && open.has(key);
    rows.push(
      node.row === 'session'
        ? { kind: 'session', key, lane: node.lane, depth, children: below.length, expanded }
        : { kind: 'absent', key, parent: node.parent, depth: 0, children: below.length, expanded },
    );
    if (expanded) for (const child of byNewest(below)) add(child, depth + 1);
  };
  for (const root of byNewest(roots)) add(root, 0);
  return { rows, grouped: children.size > 0 };
}
