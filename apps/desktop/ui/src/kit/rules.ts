import contract from '../../../../../design/rule-contract.json' with { type: 'json' };

/** One reviewed technical text registry; these definitions do not calculate metrics. */
export const rules = contract.rules;
export type RuleId = keyof typeof rules;

export function ruleText(id: RuleId): string {
  return Object.hasOwn(rules, id) ? rules[id] : 'Definition unavailable';
}

// The contract keeps stable rule keys for data and tests. Explain references
// by name when that contract text is shown to a person.
const referenceName: Record<string, string> = {
  'C-02': 'duplicate-record handling',
  'C-04': 'retry handling',
  'C-08': 'capture records',
  'M-01': 'the selected event window',
  'M-02': 'human-message classification',
  'M-03': 'assistant turns',
  'M-04': 'measured token usage',
  'M-05': 'agent time',
  'M-06': 'concurrency',
  'M-07': 'estimated human time',
  'M-08': 'the agent-to-human ratio',
  'M-09': 'hands-off stretches',
  'M-11': 'linked-session tokens',
  'M-12': 'linked-session human messages',
  'M-16': 'sessions per day',
  'M-17': 'environment usage',
  'M-18': 'session capture coverage',
  'P-01': 'metadata-only storage',
  'P-02': 'content deletion',
  'U-08': 'complete measurement verification',
};

export function displayRuleText(id: RuleId): string {
  return ruleText(id)
    .replace(
      /F20 checks independent\/new surfaces and F18 checks partial-vs-complete measurement coverage\./g,
      'Checks cover new surfaces and the difference between partial and complete measurement coverage.',
    )
    .replace(
      /F18 tests partial late-turn coverage=false, complete same-data hook coverage=true, and later unobserved usage enrichment=false again\./g,
      'Checks show that partial captures and later usage changes do not count as complete verification.',
    )
    .replace(/F18 tests/g, 'Capture tests check')
    .replace(
      /\b[A-Z]-\d{2}[a-z]?\b/g,
      (reference) => referenceName[reference] ?? 'the related rule',
    );
}
