import contract from '../../../../../design/rule-contract.json' with { type: 'json' };

/** One reviewed technical text registry; these definitions do not calculate metrics. */
export const rules = contract.rules;
export type RuleId = keyof typeof rules;

export function ruleText(id: RuleId): string {
  return Object.hasOwn(rules, id) ? rules[id] : 'Definition unavailable';
}
