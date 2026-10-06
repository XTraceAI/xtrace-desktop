import contract from '../../../../../design/rule-contract.json' with { type: 'json' };

/** One reviewed technical text registry; these definitions do not calculate metrics. */
export const rules = contract.rules;
export type RuleId = keyof typeof rules;

export function ruleText(id: RuleId): string {
  return Object.hasOwn(rules, id) ? rules[id] : 'Definition unavailable';
}

/** The longest summary a definition popover may show. */
export const SUMMARY_MAX = 160;

/**
 * What each rule's number means to the person reading it, in one everyday
 * sentence or two. This is what a definition popover shows; the full
 * technical text above stays the reviewed contract and is not shown.
 */
export const ruleSummaries: Record<RuleId, string> = {
  'C-08':
    'A note of exactly what each capture covered, so a partial capture is never treated as a complete one.',
  'M-01':
    'Only activity inside the selected range counts, even for sessions that started before it.',
  'M-02':
    'Messages counted as yours. Tool results, system notes and inputs proven to come from another agent are left out.',
  'M-03': 'One agent turn is everything an agent does between two of your messages.',
  'M-04':
    'Tokens used by each model response, counted once, and what they would cost at public API prices.',
  'M-05':
    'Time your agent sessions were active, including short pauses up to 20 minutes. Sessions running at once add up.',
  'M-06':
    'How many sessions ran at the same time: the average while any was running, and the peak.',
  'M-07':
    'An estimate of your typing time: the length of your messages, including pasted text, at your saved typing speed.',
  'M-08':
    'Agent hours divided by your hours: the time from each message you sent an agent to the next, when they are at most the break length apart.',
  'M-09':
    'How long an agent works on its own, using tools, after each of your messages before you step in again.',
  'M-10': 'The model that wrote the most output in the selected range.',
  'M-11':
    'Tokens used by the sessions linked to each merged PR. A session linked to two PRs counts for both.',
  'M-11a':
    'Agent time in the sessions linked to each merged PR. A session linked to two PRs counts for both.',
  'M-12':
    'Your messages in the sessions linked to each merged PR. A session linked to two PRs counts for both.',
  'M-12a': 'The typical hands-off stretch in the sessions linked to each merged PR.',
  'M-13':
    'How a session is tied to a pull request: a direct link, a matching commit, or a guess from the repo and branch.',
  'M-14': 'Whether a pull request is merged and how big it is, read from saved GitHub data.',
  'M-15':
    'An estimate of your typing effort: the length of your messages, including pasted text, at your saved typing speed.',
  'M-16':
    'Sessions per day, each counted on the first day it was active in the range: the average and the busiest day.',
  'M-17':
    'Which tools, MCP servers, skills, hooks, commands and subagents your agents used, and which went unused.',
  'M-18':
    'Of the sessions found for each app, how many the plugin captured. It shows whether capture works, not that every message was captured.',
  'M-19':
    'Linked pull requests merged in this range, and their effort. A session linked to several PRs is counted once.',
  'O-11':
    'Each app (command line, desktop, editor) is checked on its own, since one can stop capturing while others work.',
  'O-12':
    'For each app: sessions started and sessions captured. An app with sessions but no captures shows as not capturing.',
  'P-01':
    'New data keeps counts, names and IDs but no message text, apart from a one-line preview of your messages. Older saved text stays.',
  'P-02':
    'Deleting stored content clears saved message text but keeps the counts. Your original files are never changed.',
  'R-05': 'A record of each time a rule fired: which rule, in which session, and what it decided.',
  'R-08':
    'Tokens a reviewing model spends judging rule fires, kept separate from your session tokens.',
  'U-03': 'A tile with nothing measured shows a dash, and its note says why.',
  'U-08':
    'The share card uses the numbers you submit and never includes repo names, paths or content.',
};

/** A rule's plain-language summary, the text its definition popover shows. */
export function ruleSummary(id: RuleId): string {
  return Object.hasOwn(ruleSummaries, id) ? ruleSummaries[id] : 'Definition unavailable';
}
