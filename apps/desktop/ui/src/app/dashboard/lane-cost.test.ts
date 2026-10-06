import { expect, it } from 'vitest';
import type { DashboardLaneCost } from '../../data/generated/DashboardLaneCost';
import type { DashboardLaneSession } from '../../data/generated/DashboardLaneSession';
import type { DashboardUnpriced } from '../../data/generated/DashboardUnpriced';
import {
  mergeUnpriced,
  partialText,
  shownCost,
  shownCostText,
  unpricedNames,
} from './ActivityLanes';

const gap = (
  model: string | null,
  service_tier: string | null,
  reason: DashboardUnpriced['reason'],
  observations: number,
): DashboardUnpriced => ({ model, service_tier, reason, observations });
const cost = (unpriced: DashboardUnpriced[], priced = 0): DashboardLaneCost => {
  const missing = unpriced.reduce((sum, item) => sum + item.observations, 0);
  return {
    total_usd: null,
    priced_subtotal_usd: priced,
    selected_observations: priced + missing,
    priced_observations: priced,
    unpriced_observations: missing,
    assumed_tier_observations: 0,
    unpriced,
  };
};

it('names one unknown model once when the report lists it under two tiers', () => {
  const split = cost([
    gap('claude-new', 'standard', 'unknown_model', 2),
    gap('claude-new', 'priority', 'unknown_model', 3),
  ]);
  expect(unpricedNames(split)).toBe('claude-new has no published price');
  expect(partialText({ ...split, priced_observations: 5, selected_observations: 10 })).toBe(
    '5 of 10 responses priced; 5 claude-new have no published price',
  );
});

it('says "no model recorded" once however many tiers the report split it by', () => {
  const split = cost([
    gap(null, 'standard', 'missing_model', 1),
    gap(null, null, 'missing_model', 2),
  ]);
  expect(unpricedNames(split)).toBe('no model recorded');
  expect(partialText(split)).toBe('0 of 3 responses priced; 3 responses record no model');
});

it('merges a model’s unrecorded tiers and names unknown tiers by name', () => {
  expect(
    unpricedNames(
      cost([
        gap('gpt-x', null, 'missing_service_tier', 1),
        gap('gpt-x', '', 'missing_service_tier', 1),
        gap('gpt-x', 'flex', 'unknown_service_tier', 2),
        gap('gpt-x', 'turbo', 'unknown_service_tier', 4),
      ]),
    ),
  ).toBe(
    'gpt-x: service tier not recorded; gpt-x: service tiers flex, turbo not in the price catalog',
  );
  expect(partialText(cost([gap('gpt-x', 'flex', 'unknown_service_tier', 1)], 2))).toBe(
    '2 of 3 responses priced; 1 gpt-x response: service tier flex not in the price catalog',
  );
});

it('keeps the report’s order and sums counts per model and reason', () => {
  expect(
    mergeUnpriced([
      gap('b-model', 'standard', 'unknown_model', 1),
      gap('a-model', null, 'missing_counters', 2),
      gap('b-model', 'priority', 'unknown_model', 4),
    ]).map((item) => [item.model, item.reason, item.observations]),
  ).toEqual([
    ['b-model', 'unknown_model', 5],
    ['a-model', 'missing_counters', 2],
  ]);
});

/** A lane session carrying only the cost a sum reads. */
const session = (laneCost: DashboardLaneCost | null): DashboardLaneSession => ({
  session_id: 's',
  host: 'codex',
  repo: null,
  branch: null,
  title: null,
  automated_review: false,
  started_at_ms: null,
  pr_links: 0,
  inferred_pr_links: 0,
  cost: laneCost,
});
const whole = (usd: number, responses = 1): DashboardLaneCost => ({
  total_usd: usd,
  priced_subtotal_usd: usd,
  selected_observations: responses,
  priced_observations: responses,
  unpriced_observations: 0,
  assumed_tier_observations: 0,
  unpriced: [],
});

it('adds a group’s whole costs into one known total when every session is priced', () => {
  const sum = shownCost([session(whole(7846.14, 3)), session(whole(3.45)), session(whole(0, 0))], {
    total: true,
  });
  expect(sum.cost).toMatchObject({
    total_usd: 7849.59,
    priced_subtotal_usd: 7849.59,
    selected_observations: 4,
    priced_observations: 4,
  });
  expect(sum).toMatchObject({ total: true, sessions: 3, unknown: 0, notShown: 0 });
  expect(shownCostText(sum)).toBe('$7,849.59 API-equivalent cost of 4 responses');
});

it('keeps a group total a floor and names what an unpriced sub-session could not price', () => {
  const sum = shownCost([
    session(whole(10, 2)),
    session(cost([gap('codex-auto-review', null, 'unknown_model', 5)])),
  ]);
  expect(sum.cost).toMatchObject({
    total_usd: null,
    priced_subtotal_usd: 10,
    selected_observations: 7,
    unpriced_observations: 5,
  });
  expect(shownCostText(sum)).toBe(
    'at least $10.00 API-equivalent cost: 2 of 7 responses priced; 5 codex-auto-review have no published price',
  );
});

it('never leaves a session out of a total silently', () => {
  // A session whose cost is unknown, and sub-sessions the report left out.
  const sum = shownCost([session(whole(4, 2)), session(null), undefined], {
    total: true,
    notShown: 3,
  });
  expect(sum.cost).toMatchObject({ total_usd: null, priced_subtotal_usd: 4 });
  expect(shownCostText(sum)).toBe(
    "at least $4.00 API-equivalent cost: all 2 responses priced; 2 sessions' cost unknown and 3 more sub-sessions not shown, not included",
  );
  expect(shownCostText(shownCost([session(whole(4, 2))], { notShown: 1 }))).toBe(
    'at least $4.00 API-equivalent cost: all 2 responses priced; 1 more sub-session not shown, not included',
  );
  // Nothing read at all is unknown, never a zero.
  expect(shownCost([session(null)]).cost).toBeNull();
  expect(shownCostText(shownCost([session(null)]))).toBe('cost unknown');
  // Said once: no "cost unknown" twice.
  expect(shownCostText(shownCost([session(whole(0, 0)), session(null)], { total: true }))).toBe(
    "no responses to price; 1 session's cost unknown, not included",
  );
});

it('says a session with no responses has nothing to price, and counts an untimed one', () => {
  expect(shownCostText(shownCost([session(whole(0, 0)), session(whole(0, 0))]))).toBe(
    'no responses to price',
  );
  expect(shownCostText(shownCost([session(whole(1.5, 2))]))).toBe(
    '$1.50 API-equivalent cost of 2 responses',
  );
  // A response with no time recorded is counted and keeps the amount a floor.
  expect(
    shownCostText(shownCost([session(cost([gap('gpt-5', 'default', 'missing_timestamp', 1)], 2))])),
  ).toBe(
    'at least $2.00 API-equivalent cost: 2 of 3 responses priced; 1 gpt-5 response: no time recorded',
  );
});
