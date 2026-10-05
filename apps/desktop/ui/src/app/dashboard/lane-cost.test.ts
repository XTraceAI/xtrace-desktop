import { expect, it } from 'vitest';
import type { DashboardLaneCost } from '../../data/generated/DashboardLaneCost';
import type { DashboardUnpriced } from '../../data/generated/DashboardUnpriced';
import { mergeUnpriced, partialText, unpricedNames } from './ActivityLanes';

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
