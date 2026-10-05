import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router';
import fixture from '../fixtures/F1.json';
import { AppRoutes } from '../src/app/AppRoutes';
import * as synthetic from '../src/app/rulebook/rule-activity.synthetic';
import { DataProvider } from '../src/data/DataProvider';
import type { RuleActivityControls } from '../src/data/DataSource';
import { FixtureDataSource } from '../src/data/FixtureDataSource';
import type { FixtureExport } from '../src/data/generated/FixtureExport';
import type { RuleActivityResult } from '../src/data/generated/RuleActivityResult';
import { ThemeProvider } from '../src/theme/ThemeProvider';
import '../src/index.css';

/**
 * The app's own Shell and routes over the F1 fixture, with rule activity
 * answered from synthetic results: `?scenario=` picks the answer and `?path=`
 * the address. `hold` answers only when its read is cancelled. Every read and
 * cancel is recorded on `window.__ruleActivity` for the spec to count.
 */
const answers: Record<string, () => RuleActivityResult> = {
  populated: synthetic.loaded,
  lowerBound: synthetic.lowerBound,
  emptyExact: synthetic.emptyExact,
  emptyLowerBound: synthetic.emptyLowerBound,
  truncated: synthetic.truncated,
  lowerBoundWide: synthetic.lowerBoundWide,
  pastCap: synthetic.pastCap,
  identities: synthetic.identities,
  unavailable: () => synthetic.unavailable,
  sourceChanged: () => synthetic.sourceChanged,
  deadline: () => synthetic.deadline,
  busy: () => synthetic.busy,
  failed: () => synthetic.failed,
};
const params = new URLSearchParams(location.search);
const scenario = params.get('scenario') ?? 'populated';
const path = params.get('path') ?? '/rulebook';
const calls = { reads: [] as string[], cancels: [] as string[] };
Object.assign(window, { __ruleActivity: calls });

const held = new Map<string, (result: RuleActivityResult) => void>();
const controls: RuleActivityControls = {
  read: (id) => {
    calls.reads.push(id);
    if (scenario === 'hold') return new Promise((resolve) => held.set(id, resolve));
    const answer = answers[scenario];
    return answer ? Promise.resolve(answer()) : Promise.reject(new Error('Unknown scenario.'));
  },
  cancel: async (id) => {
    calls.cancels.push(id);
    held.get(id)?.(synthetic.interruptedCancelled);
  },
};
const source = new FixtureDataSource(fixture as FixtureExport);
Object.defineProperty(source, 'ruleActivity', { value: controls });

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <DataProvider source={source}>
        <MemoryRouter initialEntries={[path]}>
          <AppRoutes />
        </MemoryRouter>
      </DataProvider>
    </ThemeProvider>
  </StrictMode>,
);
