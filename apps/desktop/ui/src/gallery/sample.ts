// Illustrative gallery inputs only. These are not metric golden values or live observations.
export const sample = {
  label: '[SAMPLE] Illustrative component data',
  hosts: [
    { id: 'claude', label: 'Claude', count: 4 },
    { id: 'codex', label: 'Codex', count: 0 },
    { id: 'cursor', label: 'Cursor', count: null },
    { id: 'unknown-host', label: 'Unknown host', count: 2 },
  ],
  rows: [
    {
      id: 'a',
      title: 'Illustrative task A',
      detail: 'Example workspace',
      kind: 'feature' as const,
      count: 12,
      host: 'claude',
    },
    {
      id: 'b',
      title: 'Illustrative task B',
      detail: 'Example workspace',
      kind: 'bug' as const,
      count: 0,
      host: 'codex',
    },
    {
      id: 'c',
      title: 'Illustrative task C',
      detail: 'Example workspace',
      kind: 'chore' as const,
      count: null,
      host: 'unknown-host',
    },
  ],
  days: [0, 1, 3, 0, 5, 2, null, 4, 0, 2, 6, 1, 3, 0].map((value, index) => ({
    label: `Example day ${index + 1}`,
    value,
    agent: value,
    human: index % 3,
  })),
};
