import { StrictMode, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ThemeProvider, ThemeScope } from '../src/theme/ThemeProvider';
import { KindBadge, EvidenceDot, StatePill } from '../src/kit/Badge';
import { Toggle } from '../src/kit/Toggle';
import { Button } from '../src/kit/Button';
import { Segmented } from '../src/kit/Segmented';
import { ProgressBar } from '../src/kit/ProgressBar';
import { Search } from '../src/kit/Search';
import '../src/index.css';

function Controls() {
  const [checked, setChecked] = useState(false);
  const [mode, setMode] = useState('advise');
  const [search, setSearch] = useState('');
  const [saved, setSaved] = useState(0);
  const row = { display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' as const };
  return (
    <section
      aria-label="Control samples"
      style={{
        width: 680,
        padding: 24,
        display: 'flex',
        flexDirection: 'column',
        gap: 24,
        background: 'var(--canvas)',
        color: 'var(--ink)',
      }}
    >
      <h1 style={{ fontSize: 22, letterSpacing: '-0.4px' }}>Shared controls</h1>
      <p>Synthetic component states</p>
      <Search
        label="Search sessions"
        placeholder="Search sessions"
        value={search}
        onValueChange={setSearch}
      />
      <div style={row}>
        <Toggle label="Capture" checked={checked} onChange={setChecked} />
        <Button onClick={() => setSaved(saved + 1)}>Save</Button>
        <Button disabled onClick={() => setSaved(100)}>
          Unavailable
        </Button>
        <Toggle label="Locked" checked disabled onChange={() => setSaved(100)} />
      </div>
      <Segmented
        label="Rule mode"
        options={[
          { value: 'gate', label: 'Gate', tone: 'danger' },
          { value: 'advise', label: 'Advise' },
          { value: 'paused', label: 'Paused', disabled: true },
        ]}
        value={mode}
        onChange={setMode}
      />
      <output aria-label="Action count">Saved {saved}</output>
      <div style={row}>
        {(
          ['skill', 'agent', 'mcp', 'hook', 'plugin', 'cmd', 'feature', 'bug', 'chore'] as const
        ).map((kind) => (
          <KindBadge key={kind} kind={kind} />
        ))}
      </div>
      <div style={row}>
        {(['exact', 'sha', 'inferred'] as const).map((evidence) => (
          <span key={evidence}>
            <EvidenceDot evidence={evidence} /> {evidence}
          </span>
        ))}
      </div>
      <div style={row}>
        <StatePill height={20}>Unknown</StatePill>
        <StatePill tone="success" height={24} live>
          Listening
        </StatePill>
        <StatePill tone="warning" height={26}>
          Partial
        </StatePill>
        <StatePill tone="danger" outlined>
          Blocked
        </StatePill>
        <StatePill tone="info">Ready</StatePill>
        <StatePill tone="accent">Available</StatePill>
      </div>
      <div style={row}>
        {(['primary', 'accent', 'outline', 'ghost'] as const).map((variant, index) => (
          <Button key={variant} variant={variant} height={([28, 30, 34, 38] as const)[index]}>
            {variant}
          </Button>
        ))}
      </div>
      <div style={{ display: 'grid', gap: 12 }}>
        <ProgressBar label="Small progress" value={50} height={3} />
        <ProgressBar label="Complete progress" value={120} height={6} />
        <ProgressBar label="Unmeasured progress" value={null} />
        <ProgressBar
          label="Backtest"
          segments={[
            { label: 'Followed', value: 80, tone: 'success' },
            { label: 'Ignored', value: 20, tone: 'danger' },
          ]}
        />
      </div>
    </section>
  );
}
const theme = new URLSearchParams(location.search).get('theme') === 'light' ? 'light' : 'dark';
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <ThemeProvider>
      <ThemeScope theme={theme}>
        <Controls />
      </ThemeScope>
    </ThemeProvider>
  </StrictMode>,
);
