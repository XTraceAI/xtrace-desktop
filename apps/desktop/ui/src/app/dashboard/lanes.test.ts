import { expect, it } from 'vitest';
import type { DashboardLane } from '../../data/generated/DashboardLane';
import {
  groupSessionLanes,
  laneRows,
  listedLanes,
  type LaneParent,
  type SessionLane,
} from './lanes';

const lane = (
  session_id: string,
  host: string,
  start_ms: number,
  end_ms: number,
): DashboardLane => ({ session_id, host, start_ms, end_ms });

it('keeps every returned span and groups a session into one row', () => {
  const lanes = [
    lane('a', 'claude', 900, 1000),
    lane('b', 'codex', 700, 800),
    lane('a', 'claude', 100, 200),
  ];
  const grouped = groupSessionLanes(lanes);
  expect(grouped.map((row) => row.sessionId)).toEqual(['a', 'b']);
  // No span is dropped or merged: the counts still add up to the input.
  expect(grouped.reduce((total, row) => total + row.spans.length, 0)).toBe(lanes.length);
  expect(grouped[0].spans).toEqual([lanes[0], lanes[2]]);
  expect(grouped[0].firstMs).toBe(100);
  expect(grouped[0].lastMs).toBe(1000);
});

it('keeps the report order: a session sits where its most recent span did', () => {
  const grouped = groupSessionLanes([
    lane('newest', 'claude', 500, 600),
    lane('older', 'claude', 300, 400),
    lane('newest', 'claude', 100, 200),
  ]);
  expect(grouped.map((row) => row.sessionId)).toEqual(['newest', 'older']);
});

it('never merges two hosts into one row', () => {
  const grouped = groupSessionLanes([
    lane('shared', 'claude', 100, 200),
    lane('shared', 'codex', 100, 200),
  ]);
  expect(grouped).toHaveLength(2);
  expect(new Set(grouped.map((row) => row.key)).size).toBe(2);
  expect(grouped.map((row) => row.host)).toEqual(['claude', 'codex']);
});

it('reports a single-event span as its own segment', () => {
  const [row] = groupSessionLanes([lane('a', 'cursor', 400, 400)]);
  expect(row.spans).toHaveLength(1);
  expect(row.firstMs).toBe(400);
  expect(row.lastMs).toBe(400);
});

/**
 * Verified parents by child id, as the report's `lane_sessions.parent` names
 * them; a child not listed has none (unrecorded, unknown or conflicting).
 */
const parents =
  (links: Record<string, LaneParent>) =>
  (lane: SessionLane): LaneParent | null =>
    links[lane.sessionId] ?? null;
const shown = (rows: ReturnType<typeof laneRows>['rows']) =>
  rows.map((row) =>
    row.kind === 'session'
      ? `${'  '.repeat(row.depth)}${row.lane.sessionId}${row.children ? ` [${row.children}${row.expanded ? ' open' : ''}]` : ''}`
      : `absent ${row.parent.session_id} [${row.children}${row.expanded ? ' open' : ''}]`,
  );
const codex = (session_id: string): LaneParent => ({ session_id, host: 'codex' });

// Report order is newest first: c1 (newest), x, p, c2, y.
const sessions = groupSessionLanes([
  lane('c1', 'codex', 900, 1000),
  lane('x', 'codex', 800, 850),
  lane('p', 'codex', 700, 750),
  lane('c2', 'codex', 600, 650),
  lane('y', 'claude', 500, 550),
]);

it('collapses the returned children of a returned parent under its own row', () => {
  const link = parents({ c1: codex('p'), c2: codex('p') });
  const closed = laneRows(sessions, link, new Set());
  // The group sits where its newest member (c1) did; the parent keeps its own row.
  expect(shown(closed.rows)).toEqual(['p [2]', 'x', 'y']);
  expect(closed.grouped).toBe(true);
  const opened = laneRows(sessions, link, new Set([closed.rows[0].key]));
  // Children in recency order, each its own row.
  expect(shown(opened.rows)).toEqual(['p [2 open]', '  c1', '  c2', 'x', 'y']);
  expect(opened.rows[1]).toMatchObject({ kind: 'session', lane: sessions[0], depth: 1 });
});

it('names an absent parent once, at its newest returned child, with no lane of its own', () => {
  const link = parents({ c1: codex('gone'), c2: codex('gone') });
  const closed = laneRows(sessions, link, new Set());
  expect(shown(closed.rows)).toEqual(['absent gone [2]', 'x', 'p', 'y']);
  const [group] = closed.rows;
  expect(group).toMatchObject({ kind: 'absent', parent: codex('gone'), depth: 0 });
  expect('lane' in group).toBe(false);
  const opened = laneRows(sessions, link, new Set([group.key]));
  expect(shown(opened.rows)).toEqual(['absent gone [2 open]', '  c1', '  c2', 'x', 'p', 'y']);
});

it('places every returned session exactly once, and reaches each by opening groups', () => {
  const link = parents({ c1: codex('p'), c2: codex('gone'), p: codex('x') });
  const placed = (rows: ReturnType<typeof laneRows>['rows']) =>
    rows.flatMap((row) => (row.kind === 'session' ? [row.lane.key] : []));
  let open = new Set<string>();
  let rows = laneRows(sessions, link, open).rows;
  expect(shown(rows)).toEqual(['x [1]', 'absent gone [1]', 'y']);
  // Open whatever is drawn until nothing new appears.
  while (rows.some((row) => !open.has(row.key))) {
    open = new Set(rows.map((row) => row.key));
    rows = laneRows(sessions, link, open).rows;
    expect(new Set(placed(rows)).size).toBe(placed(rows).length);
  }
  expect(placed(rows).sort()).toEqual(sessions.map((lane) => lane.key).sort());
});

it('nests a returned child that has returned children of its own', () => {
  const link = parents({ c1: codex('p'), p: codex('x') });
  const x = laneRows(sessions, link, new Set()).rows[0];
  expect(shown([x])).toEqual(['x [1]']);
  const rows = laneRows(sessions, link, new Set([x.key, sessions[2].key])).rows;
  // x's group holds c1 (the newest), so it sits first.
  expect(shown(rows)).toEqual(['x [1 open]', '  p [1 open]', '    c1', 'c2', 'y']);
  // Every session under a row at any depth, open or not, in listing order;
  // a row with nothing under it has no members.
  const members = (key: string) =>
    laneRows(sessions, link, new Set())
      .rows.concat(rows)
      .find((row) => row.key === key)!
      .members.map((lane) => lane.sessionId);
  expect(members(x.key)).toEqual(['p', 'c1']);
  expect(members(sessions[2].key)).toEqual(['c1']);
  expect(members(sessions[0].key)).toEqual([]);
});

it('gives an absent parent’s row every returned session under it as members', () => {
  const link = parents({ c1: codex('gone'), c2: codex('gone'), x: codex('c2') });
  const [group] = laneRows(sessions, link, new Set()).rows;
  expect(group.kind).toBe('absent');
  expect(group.members.map((lane) => lane.sessionId)).toEqual(['c1', 'c2', 'x']);
});

it('keeps unknown, self and circular parents as ordinary rows', () => {
  const link = parents({
    // A self link is never a parent.
    x: codex('x'),
    // A loop leads nowhere reachable, so both stay ordinary rows.
    p: codex('c2'),
    c2: codex('p'),
  });
  const { rows, grouped } = laneRows(sessions, link, new Set());
  expect(grouped).toBe(false);
  expect(shown(rows)).toEqual(['c1', 'x', 'p', 'c2', 'y']);
});

it('places a session whose parents run into a loop under its own parent', () => {
  // p and c2 name each other; c1 names p but is not part of the loop.
  const link = parents({ p: codex('c2'), c2: codex('p'), c1: codex('p') });
  const { rows } = laneRows(sessions, link, new Set([sessions[2].key]));
  expect(shown(rows)).toEqual(['p [1 open]', '  c1', 'x', 'c2', 'y']);
});

it('matches a parent by host as well as id', () => {
  // The claude session named "p" is not the codex session "p".
  const link = parents({ c1: { session_id: 'p', host: 'claude' } });
  expect(shown(laneRows(sessions, link, new Set()).rows)).toEqual([
    'absent p [1]',
    'x',
    'p',
    'c2',
    'y',
  ]);
});

/** Known sub-sessions with no verified parent, by host and id. */
const unresolvedOf =
  (...known: LaneParent[]) =>
  (session: LaneParent) =>
    known.some((entry) => entry.session_id === session.session_id && entry.host === session.host);
const ids = (lanes: readonly SessionLane[]) => lanes.map((lane) => lane.sessionId);

it('lists every returned session when none is a known child with no verified parent', () => {
  const link = parents({ c1: codex('p'), c2: codex('gone') });
  expect(listedLanes(sessions, link, unresolvedOf())).toEqual(sessions);
});

it('leaves out an unresolved known child and every returned session under it', () => {
  // p is the unresolved child; c1 is under p, and c2 under c1.
  const link = parents({ c1: codex('p'), c2: codex('c1') });
  const listed = listedLanes(sessions, link, unresolvedOf(codex('p')));
  expect(ids(listed)).toEqual(['x', 'y']);
  // Nothing is left to name the hidden session as an absent parent.
  expect(shown(laneRows(listed, link, new Set()).rows)).toEqual(['x', 'y']);
});

it('leaves out a returned branch whose unreturned direct parent is an unresolved child', () => {
  const link = parents({ c1: codex('gone'), c2: codex('c1'), p: codex('kept') });
  const listed = listedLanes(sessions, link, unresolvedOf(codex('gone')));
  expect(ids(listed)).toEqual(['x', 'p', 'y']);
  // An ordinary unreturned parent still gets its one named row.
  expect(shown(laneRows(listed, link, new Set()).rows)).toEqual(['x', 'absent kept [1]', 'y']);
});

it('asks an unreturned parent only about itself, never about its own parent', () => {
  const asked: string[] = [];
  const link = parents({ c1: codex('gone') });
  const unresolved = (session: LaneParent) => {
    asked.push(session.session_id);
    return false;
  };
  expect(listedLanes(sessions, link, unresolved)).toEqual(sessions);
  expect(asked.filter((id) => id === 'gone')).toEqual(['gone']);
  expect(asked.filter((id) => !['gone', ...ids(sessions)].includes(id))).toEqual([]);
});

it('ends the walk at a loop, and matches a hidden session by host as well as id', () => {
  const loop = parents({ p: codex('c2'), c2: codex('p'), c1: codex('p') });
  expect(listedLanes(sessions, loop, unresolvedOf())).toEqual(sessions);
  // The claude session named "p" is not the codex session "p".
  const link = parents({ c1: codex('p') });
  expect(listedLanes(sessions, link, unresolvedOf({ session_id: 'p', host: 'claude' }))).toEqual(
    sessions,
  );
  // Named with its own host, the claude session "y" is left out.
  expect(
    ids(listedLanes(sessions, link, unresolvedOf({ session_id: 'y', host: 'claude' }))),
  ).toEqual(['c1', 'x', 'p', 'c2']);
});
